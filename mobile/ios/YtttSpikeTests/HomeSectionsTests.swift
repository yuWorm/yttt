import XCTest

final class HomeSectionsTests: XCTestCase {
    private func agent(_ id: String, _ state: AgentState, _ waiting: WaitingKind? = nil,
                       startedAt: UInt64) -> AgentSummary {
        AgentSummary(sessionId: id, projectId: "p", tabId: "t", paneId: id, provider: "codex",
                     state: state, waiting: waiting, waitingMessage: nil, task: nil,
                     currentAction: nil, stateStartedAtMs: startedAt, updatedAtMs: startedAt)
    }

    func testCountsAndGroups() {
        let overview = Overview(
            projects: [
                ProjectSummary(projectId: "p", name: "P", tabs: [
                    TabSummary(tabId: "a", title: "A", panes: 2),
                    TabSummary(tabId: "b", title: "B", panes: 1),
                ]),
                ProjectSummary(projectId: "q", name: "Q", tabs: []),
            ],
            agents: [
                agent("failed", .failed, startedAt: 9),
                agent("answer", .waiting, .userInput, startedAt: 8),
                agent("old-approval", .waiting, .approval, startedAt: 1),
                agent("new-approval", .waiting, .approval, startedAt: 5),
                agent("working", .working, startedAt: 3),
                agent("idle", .idle, startedAt: 4),
                agent("done", .completed, startedAt: 2),
            ],
            terminals: 6)
        let sections = HomeSections(overview)
        XCTAssertEqual([sections.projects, sections.tabs, sections.terminals, sections.agents],
                       [2, 2, 6, 7])
        XCTAssertEqual(sections.attention.map(\.sessionId),
                       ["new-approval", "old-approval", "answer", "failed"])
        XCTAssertEqual(sections.running.map(\.sessionId), ["idle", "working"])
        XCTAssertEqual(sections.finished.map(\.sessionId), ["done"])
    }

    func testNamesFallBackToIds() {
        let overview = Overview(projects: [], agents: [], terminals: 0)
        XCTAssertEqual(overview.projectName("web"), "web")
        XCTAssertEqual(overview.tabTitle(project: "web", tab: "dev"), "dev")
    }
}
