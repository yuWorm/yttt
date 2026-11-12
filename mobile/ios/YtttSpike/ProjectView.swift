import SwiftUI

/// A project: its terminal tabs, a switcher when the tab has several panes,
/// and the pane's terminal.
struct ProjectView: View {
    @EnvironmentObject private var model: AppModel
    let place: Place

    var body: some View {
        let project = model.overview.projects.first { $0.projectId == place.projectId }
        let tabs = project?.tabs ?? []
        let tab = tabs.first { $0.tabId == place.tabId }
        let panes = tab?.panes ?? []
        let pane = panes.first { $0.paneId == place.paneId } ?? panes.first
        VStack(spacing: 0) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack {
                    ForEach(tabs, id: \.tabId) { candidate in
                        let selected = candidate.tabId == tab?.tabId
                        Button(candidate.title) {
                            model.root = .project(model.overview.placeIn(place.projectId, candidate.tabId))
                        }
                        .buttonStyle(.bordered)
                        .tint(selected ? .accentColor : .secondary)
                        .fontWeight(selected ? .semibold : .regular)
                    }
                }
                .padding(.horizontal)
            }
            .padding(.vertical, 6)
            if panes.count > 1, let pane, let index = panes.firstIndex(of: pane) {
                HStack {
                    Button { go(step(panes, pane.paneId, by: -1)) } label: { Image(systemName: "chevron.left") }
                        .disabled(index == 0)
                    Text("\(index + 1)/\(panes.count) · \(pane.title)").lineLimit(1)
                    Spacer()
                    Button { go(step(panes, pane.paneId, by: 1)) } label: { Image(systemName: "chevron.right") }
                        .disabled(index == panes.count - 1)
                }
                .padding(.horizontal)
                .padding(.bottom, 6)
            }
            if let pane, let session = model.session,
               let info = model.terminals.first(where: { $0.sessionId == pane.sessionId }) {
                TerminalScreen(session: session, terminal: info,
                               tip: tipText(model.overview.agents.first { $0.sessionId == info.sessionId }))
                    .id(info.sessionId)
            } else {
                Spacer()
                Text(pane == nil ? "这个项目没有终端" : "这个 pane 没有在运行").foregroundStyle(.secondary)
                Spacer()
            }
        }
        .navigationTitle(project?.name ?? place.projectId)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button { model.drawerOpen = true } label: { Image(systemName: "line.3.horizontal") }
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button { model.drawerFilesOpen = true } label: { Image(systemName: "folder") }
            }
        }
    }

    private func go(_ paneId: String?) {
        model.root = .project(Place(projectId: place.projectId, tabId: place.tabId, paneId: paneId))
    }
}
