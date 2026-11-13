import SwiftUI

/// A pushed page: a project at a pane, or a terminal on its own.
struct DestinationView: View {
    let destination: Destination

    var body: some View {
        switch destination {
        case .project(let place): ProjectView(initial: place)
        case .terminal(let info): TerminalAloneView(info: info)
        }
    }
}

/// A project: the title menu switches project in place; under the
/// navigation bar the terminal tabs, each with its Agent's logo and a dot
/// while one waits; the pane's terminal; at the bottom a thin pane switcher
/// when the tab has several panes, above the take-control bar (or the
/// keyboard's key bar while typing).
struct ProjectView: View {
    @EnvironmentObject private var model: AppModel
    @State private var place: Place
    @State private var showFiles = false
    @State private var showTabs = false

    init(initial: Place) { _place = State(initialValue: initial) }

    var body: some View {
        let overview = model.overview
        let project = overview.projects.first { $0.projectId == place.projectId }
        let tabs = project?.tabs ?? []
        let tab = tabs.first { $0.tabId == place.tabId }
        let panes = tab?.panes ?? []
        let pane = panes.first { $0.paneId == place.paneId } ?? panes.first
        let marks = overview.tabMarks(place.projectId)
        VStack(spacing: 0) {
            LinkBanner()
            if !tabs.isEmpty {
                TabStrip(tabs: tabs, selected: tab?.tabId, marks: marks,
                         onTab: { place = overview.placeIn(place.projectId, $0) },
                         onOverview: { showTabs = true })
            }
            if let pane, let session = model.session,
               let info = model.terminals.first(where: { $0.sessionId == pane.sessionId }) {
                PaneArea(session: session, info: info,
                         agent: overview.agents.first { $0.sessionId == info.sessionId },
                         panes: panes, pane: pane, overview: overview,
                         onPane: { place = Place(projectId: place.projectId, tabId: place.tabId, paneId: $0) })
                    // A new session (a fresh connection) makes the pane again.
                    .id(model.sessionKey + info.sessionId)
            } else {
                ContentUnavailableView(pane == nil ? "这个项目没有终端" : "这个 pane 没有在运行",
                                       systemImage: "terminal")
                    .frame(maxHeight: .infinity)
                if panes.count > 1, let pane {
                    PaneBar(panes: panes, current: pane, overview: overview) {
                        place = Place(projectId: place.projectId, tabId: place.tabId, paneId: $0)
                    }
                }
            }
        }
        .background(Color.black)
        .navigationTitle(project?.name ?? place.projectId)
        .navigationBarTitleDisplayMode(.inline)
        .toolbarTitleMenu {
            ForEach(overview.projects, id: \.projectId) { candidate in
                let mark = overview.projectMark(candidate.projectId)
                Button {
                    // The current project stays where it is.
                    if candidate.projectId != place.projectId { place = overview.initialPlace(candidate.projectId) }
                } label: {
                    Label {
                        Text(candidate.name)
                        Text(projectSubtitle(mark))
                    } icon: {
                        Image(systemName: candidate.projectId == place.projectId ? "checkmark" : "folder")
                    }
                }
            }
            Divider()
            Button { model.goHome() } label: { Label("回到总控", systemImage: "square.grid.2x2") }
        }
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button { showFiles = true } label: { Image(systemName: "folder") }
                    .accessibilityLabel("文件与会话")
            }
        }
        .toolbar(.hidden, for: .tabBar)
        .toolbarBackground(.visible, for: .navigationBar)
        .sheet(isPresented: $showFiles) {
            FilesView(projectId: place.projectId)
                .environmentObject(model)
                .presentationDetents([.medium, .large])
        }
        .sheet(isPresented: $showTabs) {
            TabOverviewView(projectName: project?.name ?? place.projectId, tabs: tabs, selected: tab?.tabId,
                            marks: marks, overview: overview) { tabId in
                showTabs = false
                place = overview.placeIn(place.projectId, tabId)
            }
        }
        .onAppear {
            if model.autoFilesPending {
                model.autoFilesPending = false
                showFiles = true
            }
        }
    }

    private func projectSubtitle(_ mark: ProjectMark) -> String {
        var parts: [String] = []
        if mark.waiting > 0 { parts.append("\(mark.waiting) 个在等") }
        if !mark.providers.isEmpty { parts.append(mark.providers.map(providerName).joined(separator: "、")) }
        return parts.isEmpty ? "没有运行中的 Agent" : parts.joined(separator: " · ")
    }
}

/// The project's terminal tabs as capsules, scrolled to the selected one;
/// the last button opens the tab overview.
private struct TabStrip: View {
    let tabs: [TabSummary]
    let selected: String?
    let marks: [String: TabMark]
    let onTab: (String) -> Void
    let onOverview: () -> Void

    var body: some View {
        // The tabs scroll in their own space beside the button, never under it,
        // and the whole strip shares one background.
        HStack(spacing: 0) {
            ScrollViewReader { reader in
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(tabs, id: \.tabId) { tab in
                            let mark = marks[tab.tabId]
                            let label = HStack(spacing: 5) {
                                if let provider = mark?.provider {
                                    AgentLogo(provider: provider, size: 14)
                                } else {
                                    Image(systemName: "terminal").font(.caption)
                                }
                                Text(tab.title).lineLimit(1)
                                if mark?.waiting == true {
                                    Circle().fill(.orange).frame(width: 7, height: 7)
                                }
                            }
                            if tab.tabId == selected {
                                Button { onTab(tab.tabId) } label: { label }
                                    .buttonStyle(.borderedProminent)
                                    .id(tab.tabId)
                            } else {
                                Button { onTab(tab.tabId) } label: { label }
                                    .buttonStyle(.bordered)
                                    .tint(.primary)
                                    .id(tab.tabId)
                            }
                        }
                    }
                    .buttonBorderShape(.capsule)
                    .controlSize(.small)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                }
                .onAppear { if let selected { reader.scrollTo(selected, anchor: .center) } }
                .onChange(of: selected) { _, id in
                    if let id { withAnimation { reader.scrollTo(id, anchor: .center) } }
                }
            }
            Button(action: onOverview) { Image(systemName: "square.grid.2x2") }
                .accessibilityLabel("全部 Tab")
                .padding(.horizontal, 12)
        }
        .fixedSize(horizontal: false, vertical: true)
        .background(.bar)
        .overlay(alignment: .bottom) { Divider() }
    }
}

/// One pane: its terminal, the pane switcher, and the take-control bar.
private struct PaneArea: View {
    let info: TerminalInfo
    let agent: AgentSummary?
    let panes: [PaneSummary]
    let pane: PaneSummary
    let overview: Overview
    let onPane: (String?) -> Void
    @StateObject private var control: TerminalControl

    init(session: HostSession, info: TerminalInfo, agent: AgentSummary?, panes: [PaneSummary], pane: PaneSummary,
         overview: Overview, onPane: @escaping (String?) -> Void) {
        self.info = info
        self.agent = agent
        self.panes = panes
        self.pane = pane
        self.overview = overview
        self.onPane = onPane
        _control = StateObject(wrappedValue: TerminalControl(session: session, sessionId: info.sessionId))
    }

    var body: some View {
        VStack(spacing: 0) {
            TerminalScreen(terminal: info, control: control)
                .frame(maxHeight: .infinity)
            if panes.count > 1 {
                PaneBar(panes: panes, current: pane, overview: overview, onPane: onPane)
            }
            if !control.controlling {
                TakeControlBar(agent: agent, control: control)
            }
        }
        .toolbar {
            if control.controlling {
                if control.keyboardShown {
                    ToolbarItem(placement: .topBarTrailing) { HideKeyboardButton(control: control) }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button { control.giveBack() } label: { Label("交还电脑", systemImage: "desktopcomputer") }
                        .labelStyle(.titleOnly)
                }
            }
        }
    }
}

/// The tab's panes in a thin bar, tapped or swiped through.
private struct PaneBar: View {
    let panes: [PaneSummary]
    let current: PaneSummary
    let overview: Overview
    let onPane: (String?) -> Void

    var body: some View {
        let index = panes.firstIndex(of: current) ?? 0
        HStack(spacing: 6) {
            Image(systemName: "rectangle.split.2x1").font(.system(size: 11)).foregroundStyle(.secondary)
            ForEach(panes, id: \.paneId) { candidate in
                let selected = candidate.paneId == current.paneId
                let provider = overview.agents.first { $0.sessionId == candidate.sessionId }?.provider
                Button { onPane(candidate.paneId) } label: {
                    HStack(spacing: 4) {
                        if let provider { AgentLogo(provider: provider, size: 10) }
                        Text(candidate.title).lineLimit(1)
                    }
                    .font(.system(size: 11, weight: selected ? .semibold : .regular))
                    .padding(.horizontal, 8)
                    .frame(height: 19)
                    .background(selected ? Color.white.opacity(0.2) : .clear, in: Capsule())
                }
                .foregroundStyle(selected ? .white : Color(white: 0.82))
            }
            Spacer(minLength: 0)
            Text("\(index + 1) / \(panes.count)")
                .font(.system(size: 11).monospacedDigit())
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 10)
        .frame(height: 26)
        .background(Color(white: 0.11))
        .environment(\.colorScheme, .dark)
        .contentShape(Rectangle())
        .gesture(
            DragGesture(minimumDistance: 20).onEnded { value in
                if value.translation.width < -40 { onPane(step(panes, current.paneId, by: 1)) }
                if value.translation.width > 40 { onPane(step(panes, current.paneId, by: -1)) }
            }
        )
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Pane \(index + 1) / \(panes.count)")
    }
}

/// All the project's tabs as cards, for when the strip has too many to scan.
private struct TabOverviewView: View {
    let projectName: String
    let tabs: [TabSummary]
    let selected: String?
    let marks: [String: TabMark]
    let overview: Overview
    let onTab: (String) -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)], spacing: 12) {
                    ForEach(tabs, id: \.tabId) { tab in
                        Button { onTab(tab.tabId) } label: { card(tab) }
                            .buttonStyle(.plain)
                    }
                }
                .padding()
            }
            .background(Color(uiColor: .systemGroupedBackground))
            .navigationTitle("\(projectName) 的 \(tabs.count) 个 Tab")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("完成") { dismiss() } }
            }
        }
    }

    private func card(_ tab: TabSummary) -> some View {
        let mark = marks[tab.tabId]
        let agents = tab.panes.compactMap { pane in overview.agents.first { $0.sessionId == pane.sessionId } }
        let lead = agents.first { $0.state == .waiting } ?? agents.first
        return VStack(alignment: .leading, spacing: 0) {
            ZStack {
                Color.black
                if let provider = mark?.provider {
                    AgentLogo(provider: provider, size: 34).foregroundStyle(.white)
                } else {
                    Image(systemName: "terminal").font(.system(size: 30)).foregroundStyle(.white.opacity(0.8))
                }
            }
            .frame(height: 84)
            .overlay(alignment: .topTrailing) {
                if tab.panes.count > 1 {
                    Text("\(tab.panes.count) 个 pane")
                        .font(.caption2).foregroundStyle(.white)
                        .padding(.horizontal, 6).padding(.vertical, 2)
                        .background(.black.opacity(0.6), in: Capsule())
                        .padding(6)
                }
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(tab.title).font(.subheadline.weight(.semibold)).lineLimit(1)
                if let lead {
                    Label(lead.stateLabel, systemImage: lead.stateSymbol)
                        .font(.caption.weight(.medium)).foregroundStyle(lead.stateColor)
                } else {
                    Text("终端").font(.caption).foregroundStyle(.secondary)
                }
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color(uiColor: .secondarySystemGroupedBackground))
        }
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay {
            if tab.tabId == selected {
                RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Color.accentColor, lineWidth: 3)
            }
        }
    }
}

/// A terminal on its own, when it runs in no pane.
struct TerminalAloneView: View {
    @EnvironmentObject private var model: AppModel
    let info: TerminalInfo

    var body: some View {
        VStack(spacing: 0) {
            LinkBanner()
            if let session = model.session, model.link != .ready || model.terminals.contains(where: { $0.sessionId == info.sessionId }) {
                AloneArea(session: session, info: info,
                          agent: model.overview.agents.first { $0.sessionId == info.sessionId })
                    .id(model.sessionKey + info.sessionId)
            } else {
                ContentUnavailableView("终端已结束", systemImage: "terminal",
                                       description: Text("电脑上的这个终端已经不在了。"))
            }
        }
        .navigationTitle(info.sessionId)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
    }
}

private struct AloneArea: View {
    let info: TerminalInfo
    let agent: AgentSummary?
    @StateObject private var control: TerminalControl

    init(session: HostSession, info: TerminalInfo, agent: AgentSummary?) {
        self.info = info
        self.agent = agent
        _control = StateObject(wrappedValue: TerminalControl(session: session, sessionId: info.sessionId))
    }

    var body: some View {
        VStack(spacing: 0) {
            TerminalScreen(terminal: info, control: control).frame(maxHeight: .infinity)
            if !control.controlling { TakeControlBar(agent: agent, control: control) }
        }
        .toolbar {
            if control.controlling {
                if control.keyboardShown {
                    ToolbarItem(placement: .topBarTrailing) { HideKeyboardButton(control: control) }
                }
                ToolbarItem(placement: .topBarTrailing) { Button("交还电脑") { control.giveBack() } }
            }
        }
    }
}

/// Puts the system keyboard away while typing into a terminal.
private struct HideKeyboardButton: View {
    let control: TerminalControl

    var body: some View {
        Button { control.hideKeyboard() } label: { Image(systemName: "keyboard.chevron.compact.down") }
            .accessibilityLabel("收起键盘")
    }
}
