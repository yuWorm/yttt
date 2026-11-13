package com.yttt.yai

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import uniffi.yttt_mobile_core.AgentState
import uniffi.yttt_mobile_core.AgentSummary
import uniffi.yttt_mobile_core.Overview
import uniffi.yttt_mobile_core.PaneState
import uniffi.yttt_mobile_core.PaneSummary
import uniffi.yttt_mobile_core.ProjectSummary
import uniffi.yttt_mobile_core.TabSummary
import uniffi.yttt_mobile_core.TerminalInfo
import uniffi.yttt_mobile_core.WaitingKind

class ProjectPageTest {
    private fun pane(tab: String, id: String) = PaneSummary(id, id.uppercase(), "web:$tab:$id", PaneState.RUNNING)

    private fun agent(tab: String, pane: String, state: AgentState, waiting: WaitingKind? = null, project: String = "web") =
        AgentSummary(
            sessionId = "$project:$tab:$pane", projectId = project, tabId = "$project:$tab:$pane",
            paneId = "$project:$tab:$pane", provider = "codex", state = state, waiting = waiting,
            waitingMessage = null, task = null, currentAction = null, stateStartedAtMs = 1u, updatedAtMs = 1u,
        )

    private val overview = Overview(
        projects = listOf(
            ProjectSummary(
                projectId = "web", name = "Website", selectedTabId = "dev",
                tabs = listOf(
                    TabSummary("dev", "Dev", listOf(pane("dev", "server"), pane("dev", "logs")), focusedPaneId = "logs"),
                    TabSummary("agents", "Agents", listOf(pane("agents", "codex")), focusedPaneId = null),
                ),
            ),
            ProjectSummary("api", "API", emptyList(), selectedTabId = "main"),
        ),
        agents = listOf(
            agent("agents", "codex", AgentState.WAITING, WaitingKind.APPROVAL),
            agent("dev", "server", AgentState.FAILED),
            agent("dev", "logs", AgentState.WORKING),
            agent("x", "y", AgentState.IDLE, project = "api"),
            agent("x", "z", AgentState.COMPLETED, project = "api"),
        ),
        terminals = 3u,
        terminalIds = emptyList(),
    )

    @Test
    fun badgesCountWaitingFailedAndRunningPerProject() {
        val badges = overview.badges()
        assertEquals(Badges(waiting = 1, failed = 1, running = 1), badges["web"])
        assertEquals(Badges(waiting = 0, failed = 0, running = 1), badges["api"])
    }

    @Test
    fun aProjectOpensOnTheDesktopsTabAndFocusedPane() {
        assertEquals(Place("web", "dev", "logs"), overview.initialPlace("web"))
        // Without a focused pane, the first; without the selected tab, the first tab.
        assertEquals(Place("web", "agents", "codex"), overview.placeIn("web", "agents"))
        assertEquals(Place("api", null, null), overview.initialPlace("api"))
    }

    @Test
    fun anAgentOrTerminalOpensWhereItRuns() {
        assertEquals(Place("web", "agents", "codex"), overview.placeOf("web:agents:codex"))
        assertNull(overview.placeOf("gone:tab:pane"))
    }

    @Test
    fun panesStepWithinTheTabAndStopAtTheEnds() {
        val panes = overview.projects[0].tabs[0].panes
        assertEquals("logs", step(panes, "server", +1))
        assertEquals("logs", step(panes, "logs", +1))
        assertEquals("server", step(panes, "logs", -1))
        assertEquals("server", step(panes, "server", -1))
    }

    @Test
    fun theTipSaysWhatAWaitingAgentWants() {
        assertEquals("接管终端", tipText(null))
        assertEquals("接管终端", tipText(agent("dev", "logs", AgentState.WORKING)))
        assertEquals("等批准，接管后回复", tipText(agent("a", "b", AgentState.WAITING, WaitingKind.APPROVAL)))
        assertEquals("等回答，接管后回复", tipText(agent("a", "b", AgentState.WAITING, WaitingKind.USER_INPUT)))
    }

    @Test
    fun aTabShowsItsAgentsLogoAndWhetherOneWaits() {
        val marks = overview.tabMarks("web")
        assertEquals(TabMark(provider = "codex", waiting = true), marks["agents"])
        assertEquals(TabMark(provider = "codex", waiting = false), marks["dev"])
        assertEquals(emptyMap<String, TabMark>(), overview.tabMarks("api"))
    }

    @Test
    fun aProjectListsItsActiveAgentsOnceAndCountsWaits() {
        val withClaude = overview.copy(
            agents = overview.agents + agent("agents", "codex", AgentState.WORKING).copy(sessionId = "c2", provider = "claude"),
        )
        assertEquals(ProjectMark(providers = listOf("codex", "claude"), waiting = 1), withClaude.projectMark("web"))
        // Finished Agents are not shown.
        assertEquals(ProjectMark(providers = listOf("codex"), waiting = 0), overview.projectMark("api"))
        assertEquals(ProjectMark(emptyList(), 0), overview.projectMark("none"))
    }

    @Test
    fun anAgentOpensItsPaneElseItsTerminalElseNothing() {
        val terminals = listOf(TerminalInfo("loose", "api", 80.toUShort(), 24.toUShort()))
        assertEquals(Destination.Project(Place("web", "agents", "codex")), destination("web:agents:codex", overview, terminals))
        assertEquals(Destination.Terminal(terminals[0]), destination("loose", overview, terminals))
        assertNull(destination("gone", overview, terminals))
    }
}
