import XCTest

final class ProjectPageTests: XCTestCase {
    private func pane(_ tab: String, _ id: String) -> PaneSummary {
        PaneSummary(paneId: id, title: id.uppercased(), sessionId: "web:\(tab):\(id)", state: .running)
    }

    private func agent(_ tab: String, _ pane: String, _ state: AgentState, _ waiting: WaitingKind? = nil,
                       project: String = "web") -> AgentSummary {
        let id = "\(project):\(tab):\(pane)"
        return AgentSummary(sessionId: id, projectId: project, tabId: id, paneId: id, provider: "codex",
                            state: state, waiting: waiting, waitingMessage: nil, task: nil,
                            currentAction: nil, stateStartedAtMs: 1, updatedAtMs: 1)
    }

    private lazy var overview = Overview(
        projects: [
            ProjectSummary(projectId: "web", name: "Website", tabs: [
                TabSummary(tabId: "dev", title: "Dev", panes: [pane("dev", "server"), pane("dev", "logs")],
                           focusedPaneId: "logs"),
                TabSummary(tabId: "agents", title: "Agents", panes: [pane("agents", "codex")], focusedPaneId: nil),
            ], selectedTabId: "dev"),
            ProjectSummary(projectId: "api", name: "API", tabs: [], selectedTabId: "main"),
        ],
        agents: [
            agent("agents", "codex", .waiting, .approval),
            agent("dev", "server", .failed),
            agent("dev", "logs", .working),
            agent("x", "y", .idle, project: "api"),
            agent("x", "z", .completed, project: "api"),
        ],
        terminals: 3)

    func testBadgesCountWaitingFailedAndRunningPerProject() {
        let badges = overview.badges()
        XCTAssertEqual(badges["web"], Badges(waiting: 1, failed: 1, running: 1))
        XCTAssertEqual(badges["api"], Badges(waiting: 0, failed: 0, running: 1))
    }

    func testAProjectOpensOnTheDesktopsTabAndFocusedPane() {
        XCTAssertEqual(overview.initialPlace("web"), Place(projectId: "web", tabId: "dev", paneId: "logs"))
        XCTAssertEqual(overview.placeIn("web", "agents"), Place(projectId: "web", tabId: "agents", paneId: "codex"))
        XCTAssertEqual(overview.initialPlace("api"), Place(projectId: "api", tabId: nil, paneId: nil))
    }

    func testAnAgentOrTerminalOpensWhereItRuns() {
        XCTAssertEqual(overview.placeOf("web:agents:codex"), Place(projectId: "web", tabId: "agents", paneId: "codex"))
        XCTAssertNil(overview.placeOf("gone:tab:pane"))
    }

    func testPanesStepWithinTheTabAndStopAtTheEnds() {
        let panes = overview.projects[0].tabs[0].panes
        XCTAssertEqual(step(panes, "server", by: 1), "logs")
        XCTAssertEqual(step(panes, "logs", by: 1), "logs")
        XCTAssertEqual(step(panes, "logs", by: -1), "server")
        XCTAssertEqual(step(panes, "server", by: -1), "server")
    }

    func testTheTipSaysWhatAWaitingAgentWants() {
        XCTAssertEqual(tipText(nil), "接管终端")
        XCTAssertEqual(tipText(agent("dev", "logs", .working)), "接管终端")
        XCTAssertEqual(tipText(agent("a", "b", .waiting, .approval)), "等批准，接管后回复")
        XCTAssertEqual(tipText(agent("a", "b", .waiting, .userInput)), "等回答，接管后回复")
    }

    func testATabShowsItsAgentsLogoAndWhetherOneWaits() {
        let marks = overview.tabMarks("web")
        XCTAssertEqual(marks["agents"], TabMark(provider: "codex", waiting: true))
        XCTAssertEqual(marks["dev"], TabMark(provider: "codex", waiting: false))
        XCTAssertEqual(overview.tabMarks("api"), [:])
    }

    func testAProjectListsItsActiveAgentsOnceAndCountsWaits() {
        var claude = agent("agents", "codex", .working)
        claude.sessionId = "c2"
        claude.provider = "claude"
        var withClaude = overview
        withClaude.agents.append(claude)
        XCTAssertEqual(withClaude.projectMark("web"), ProjectMark(providers: ["codex", "claude"], waiting: 1))
        // Finished Agents are not shown.
        XCTAssertEqual(overview.projectMark("api"), ProjectMark(providers: ["codex"], waiting: 0))
        XCTAssertEqual(overview.projectMark("none"), ProjectMark(providers: [], waiting: 0))
    }

    func testAnAgentOpensItsPaneElseItsTerminalElseNothing() {
        let terminals = [TerminalInfo(sessionId: "loose", projectId: "api", cols: 80, rows: 24)]
        XCTAssertEqual(destination("web:agents:codex", overview, terminals),
                       .project(Place(projectId: "web", tabId: "agents", paneId: "codex")))
        XCTAssertEqual(destination("loose", overview, terminals), .terminal(terminals[0]))
        XCTAssertNil(destination("gone", overview, terminals))
    }
}
