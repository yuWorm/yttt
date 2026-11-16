import Foundation

/// The home screen's groups, counted and sorted natively from the core's
/// overview: what runs right now, then every Agent under its project, as the
/// desktop sidebar groups them.
struct HomeSections {
    /// A project's Agents, most urgent first.
    struct ProjectGroup {
        let projectId: String
        let name: String
        let agents: [AgentSummary]
    }

    let projects: Int
    let tabs: Int
    let terminals: Int
    let agents: Int
    /// Waiting for approval, then for an answer, then for anything else,
    /// then failed; most recent change first within each.
    let attention: [AgentSummary]
    /// Working or starting; an idle Agent is not running.
    let running: [AgentSummary]
    /// Projects in the Host's order, then Agents of projects no longer
    /// listed; projects without Agents are left out.
    let groups: [ProjectGroup]

    init(_ overview: Overview) {
        projects = overview.projects.count
        tabs = overview.projects.reduce(0) { $0 + $1.tabs.count }
        terminals = Int(overview.terminals)
        agents = overview.agents.count
        let urgentFirst: (AgentSummary, AgentSummary) -> Bool = {
            let (left, right) = (Self.rank($0), Self.rank($1))
            return left != right ? left < right : $0.stateStartedAtMs > $1.stateStartedAtMs
        }
        attention = overview.agents.filter { Self.attentionRank($0) != nil }.sorted(by: urgentFirst)
        running = overview.agents.filter { [.working, .starting].contains($0.state) }.sorted(by: urgentFirst)

        let byProject = Dictionary(grouping: overview.agents, by: \.projectId)
        let listed = overview.projects.map { ($0.projectId, $0.name) }
        let known = Set(listed.map(\.0))
        var unlisted: [(String, String)] = []
        for agent in overview.agents where !known.contains(agent.projectId)
            && !unlisted.contains(where: { $0.0 == agent.projectId }) {
            unlisted.append((agent.projectId, agent.projectId))
        }
        groups = (listed + unlisted).compactMap { id, name in
            guard let agents = byProject[id] else { return nil }
            return ProjectGroup(projectId: id, name: name, agents: agents.sorted(by: urgentFirst))
        }
    }

    /// Needs the user (0–3), runs (4), idles (5), ended (6).
    static func rank(_ agent: AgentSummary) -> Int {
        if let rank = attentionRank(agent) { return rank }
        switch agent.state {
        case .working, .starting: return 4
        case .idle: return 5
        default: return 6
        }
    }

    static func attentionRank(_ agent: AgentSummary) -> Int? {
        switch agent.state {
        case .waiting:
            switch agent.waiting {
            case .approval: return 0
            case .userInput: return 1
            default: return 2
            }
        case .failed: return 3
        default: return nil
        }
    }
}

extension Overview {
    func projectName(_ id: String) -> String {
        projects.first { $0.projectId == id }?.name ?? id
    }

    func tabTitle(project: String, tab: String) -> String {
        projects.first { $0.projectId == project }?.tabs.first { $0.tabId == tab }?.title ?? tab
    }
}

extension AgentSummary {
    var stateLabel: String {
        switch state {
        case .waiting:
            switch waiting {
            case .approval: return "等批准"
            case .userInput: return "等回答"
            default: return "等待"
            }
        case .failed: return "失败"
        case .working: return "工作中"
        case .starting: return "启动中"
        case .idle: return "空闲"
        case .completed: return "完成"
        case .interrupted: return "已中断"
        case .stale: return "无响应"
        }
    }

    /// How long the Agent has been in its state, e.g. "3 分钟".
    func elapsed(now: Date = Date()) -> String {
        let seconds = max(0, Int(now.timeIntervalSince1970) - Int(stateStartedAtMs / 1000))
        if seconds < 60 { return "\(seconds) 秒" }
        if seconds < 3600 { return "\(seconds / 60) 分钟" }
        return "\(seconds / 3600) 小时"
    }
}
