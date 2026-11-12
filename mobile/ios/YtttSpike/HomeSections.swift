import Foundation

/// The home screen's groups, counted and sorted natively from the core's
/// overview: what needs the user first, then what runs, then what ended.
struct HomeSections {
    let projects: Int
    let tabs: Int
    let terminals: Int
    let agents: Int
    /// Waiting for approval, then for an answer, then for anything else,
    /// then failed; most recent change first within each.
    let attention: [AgentSummary]
    let running: [AgentSummary]
    let finished: [AgentSummary]

    init(_ overview: Overview) {
        projects = overview.projects.count
        tabs = overview.projects.reduce(0) { $0 + $1.tabs.count }
        terminals = Int(overview.terminals)
        agents = overview.agents.count
        let recentFirst: (AgentSummary, AgentSummary) -> Bool = {
            $0.stateStartedAtMs > $1.stateStartedAtMs
        }
        attention = overview.agents
            .filter { Self.attentionRank($0) != nil }
            .sorted {
                let (left, right) = (Self.attentionRank($0)!, Self.attentionRank($1)!)
                return left != right ? left < right : recentFirst($0, $1)
            }
        running = overview.agents
            .filter { [.working, .starting, .idle].contains($0.state) }
            .sorted(by: recentFirst)
        finished = overview.agents
            .filter { [.completed, .interrupted, .stale].contains($0.state) }
            .sorted(by: recentFirst)
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
