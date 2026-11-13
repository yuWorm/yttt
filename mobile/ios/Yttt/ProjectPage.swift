import Foundation

/// A project's counts in the left drawer.
struct Badges: Equatable {
    var waiting = 0
    var failed = 0
    var running = 0
}

/// Where the project page shows: a project, and in it a tab and a pane.
struct Place: Equatable, Hashable {
    let projectId: String
    let tabId: String?
    let paneId: String?
}

extension Overview {
    func badges() -> [String: Badges] {
        var badges: [String: Badges] = [:]
        for agent in agents {
            var badge = badges[agent.projectId, default: Badges()]
            switch agent.state {
            case .waiting: badge.waiting += 1
            case .failed: badge.failed += 1
            case .working, .starting, .idle: badge.running += 1
            default: break
            }
            badges[agent.projectId] = badge
        }
        return badges
    }

    /// A project as the desktop shows it: its selected tab, that tab's focused pane.
    func initialPlace(_ projectId: String) -> Place {
        guard let project = projects.first(where: { $0.projectId == projectId }),
              let tab = project.tabs.first(where: { $0.tabId == project.selectedTabId }) ?? project.tabs.first
        else { return Place(projectId: projectId, tabId: nil, paneId: nil) }
        return placeIn(projectId, tab.tabId)
    }

    /// A tab opens on its focused pane, or its first.
    func placeIn(_ projectId: String, _ tabId: String) -> Place {
        let tab = projects.first { $0.projectId == projectId }?.tabs.first { $0.tabId == tabId }
        let pane = tab?.panes.first { $0.paneId == tab?.focusedPaneId } ?? tab?.panes.first
        return Place(projectId: projectId, tabId: tab?.tabId, paneId: pane?.paneId)
    }

    /// The pane running terminal `sessionId`, as for an Agent or a notification.
    func placeOf(_ sessionId: String) -> Place? {
        for project in projects {
            for tab in project.tabs {
                if let pane = tab.panes.first(where: { $0.sessionId == sessionId }) {
                    return Place(projectId: project.projectId, tabId: tab.tabId, paneId: pane.paneId)
                }
            }
        }
        return nil
    }
}

/// The pane `by` steps from `current` within `panes`, staying at the ends.
func step(_ panes: [PaneSummary], _ current: String?, by: Int) -> String? {
    guard !panes.isEmpty else { return nil }
    let index = panes.firstIndex { $0.paneId == current } ?? 0
    return panes[min(max(index + by, 0), panes.count - 1)].paneId
}

/// The take-control tip, saying what the pane's Agent waits for.
func tipText(_ agent: AgentSummary?) -> String {
    guard let agent, agent.state == .waiting else { return "接管终端" }
    switch agent.waiting {
    case .approval: return "等批准，接管后回复"
    case .userInput: return "等回答，接管后回复"
    default: return "等待中，接管后处理"
    }
}

/// A terminal tab's mark: the logo of an Agent in it, and whether one waits.
struct TabMark: Equatable {
    var provider: String?
    var waiting: Bool
}

/// A project's mark: its active Agents' providers, once each, and how many wait.
struct ProjectMark: Equatable {
    var providers: [String]
    var waiting: Int
}

extension AgentSummary {
    var isActive: Bool { ![.completed, .interrupted, .stale].contains(state) }
}

extension Overview {
    /// Each tab of the project with an Agent in its panes: a waiting one's provider first.
    func tabMarks(_ projectId: String) -> [String: TabMark] {
        guard let project = projects.first(where: { $0.projectId == projectId }) else { return [:] }
        let bySession = Dictionary(agents.map { ($0.sessionId, $0) }, uniquingKeysWith: { first, _ in first })
        var marks: [String: TabMark] = [:]
        for tab in project.tabs {
            let inTab = tab.panes.compactMap { bySession[$0.sessionId] }
            guard let first = inTab.first else { continue }
            let waiting = inTab.first { $0.state == .waiting }
            marks[tab.tabId] = TabMark(provider: (waiting ?? first).provider, waiting: waiting != nil)
        }
        return marks
    }

    func projectMark(_ projectId: String) -> ProjectMark {
        let inProject = agents.filter { $0.projectId == projectId }
        var providers: [String] = []
        for agent in inProject where agent.isActive && !providers.contains(agent.provider) {
            providers.append(agent.provider)
        }
        return ProjectMark(providers: providers, waiting: inProject.filter { $0.state == .waiting }.count)
    }
}

/// Where tapping an Agent leads.
enum Destination: Hashable {
    case project(Place)
    case terminal(TerminalInfo)
}

/// The pane the terminal runs in, else the terminal alone, else nothing: it has gone.
func destination(_ sessionId: String, _ overview: Overview, _ terminals: [TerminalInfo]) -> Destination? {
    if let place = overview.placeOf(sessionId) { return .project(place) }
    return terminals.first { $0.sessionId == sessionId }.map { .terminal($0) }
}
