package com.yttt.yai

import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.yttt_mobile_core.AgentState
import uniffi.yttt_mobile_core.AgentSummary
import uniffi.yttt_mobile_core.Overview
import uniffi.yttt_mobile_core.ProjectSummary
import uniffi.yttt_mobile_core.TabSummary
import uniffi.yttt_mobile_core.WaitingKind

class HomeSectionsTest {
    private fun agent(id: String, state: AgentState, waiting: WaitingKind? = null, startedAt: Long, project: String = "p") =
        AgentSummary(
            sessionId = id, projectId = project, tabId = "t", paneId = id, provider = "codex",
            state = state, waiting = waiting, waitingMessage = null, task = null,
            currentAction = null, stateStartedAtMs = startedAt.toULong(), updatedAtMs = startedAt.toULong(),
        )

    @Test
    fun countsAndGroups() {
        val overview = Overview(
            projects = listOf(
                ProjectSummary("p", "P", listOf(TabSummary("a", "A", emptyList(), null), TabSummary("b", "B", emptyList(), null)), "a"),
                ProjectSummary("q", "Q", emptyList(), "main"),
            ),
            agents = listOf(
                agent("failed", AgentState.FAILED, startedAt = 9),
                agent("answer", AgentState.WAITING, WaitingKind.USER_INPUT, 8),
                agent("old-approval", AgentState.WAITING, WaitingKind.APPROVAL, 1),
                agent("new-approval", AgentState.WAITING, WaitingKind.APPROVAL, 5),
                agent("working", AgentState.WORKING, startedAt = 3),
                agent("idle", AgentState.IDLE, startedAt = 4),
                agent("done", AgentState.COMPLETED, startedAt = 2),
            ),
            terminals = 6u,
            terminalIds = emptyList(),
        )
        val sections = HomeSections(overview)
        assertEquals(listOf(2, 2, 6, 7), listOf(sections.projects, sections.tabs, sections.terminals, sections.agents))
        assertEquals(listOf("new-approval", "old-approval", "answer", "failed"), sections.attention.map { it.sessionId })
        assertEquals(listOf("working"), sections.running.map { it.sessionId })
    }

    @Test
    fun runningLeavesIdleOut() {
        val overview = Overview(
            emptyList(),
            listOf(
                agent("idle", AgentState.IDLE, startedAt = 9),
                agent("starting", AgentState.STARTING, startedAt = 2),
                agent("working", AgentState.WORKING, startedAt = 1),
            ),
            0u,
            emptyList(),
        )
        assertEquals(listOf("starting", "working"), HomeSections(overview).running.map { it.sessionId })
    }

    @Test
    fun groupsFollowProjectOrderAndSortByUrgency() {
        val overview = Overview(
            projects = listOf(
                ProjectSummary("q", "Q", emptyList(), "main"),
                ProjectSummary("empty", "E", emptyList(), "main"),
                ProjectSummary("p", "P", emptyList(), "main"),
            ),
            agents = listOf(
                agent("p-done", AgentState.COMPLETED, startedAt = 9, project = "p"),
                agent("p-idle", AgentState.IDLE, startedAt = 8, project = "p"),
                agent("p-working", AgentState.WORKING, startedAt = 1, project = "p"),
                agent("p-approval", AgentState.WAITING, WaitingKind.APPROVAL, 2, project = "p"),
                agent("p-working-new", AgentState.WORKING, startedAt = 5, project = "p"),
                agent("q-idle", AgentState.IDLE, startedAt = 1, project = "q"),
                agent("gone", AgentState.WORKING, startedAt = 1, project = "gone"),
            ),
            terminals = 0u,
            terminalIds = emptyList(),
        )
        val groups = HomeSections(overview).groups
        assertEquals(listOf("q", "p", "gone"), groups.map { it.projectId })
        assertEquals(listOf("Q", "P", "gone"), groups.map { it.name })
        assertEquals(
            listOf("p-approval", "p-working-new", "p-working", "p-idle", "p-done"),
            groups[1].agents.map { it.sessionId },
        )
    }

    @Test
    fun namesFallBackToIds() {
        val overview = Overview(emptyList(), emptyList(), 0u, emptyList())
        assertEquals("web", overview.projectName("web"))
        assertEquals("dev", overview.tabTitle("web", "dev"))
    }
}
