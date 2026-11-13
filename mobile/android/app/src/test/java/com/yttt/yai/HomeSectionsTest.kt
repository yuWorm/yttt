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
    private fun agent(id: String, state: AgentState, waiting: WaitingKind? = null, startedAt: Long) =
        AgentSummary(
            sessionId = id, projectId = "p", tabId = "t", paneId = id, provider = "codex",
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
        )
        val sections = HomeSections(overview)
        assertEquals(listOf(2, 2, 6, 7), listOf(sections.projects, sections.tabs, sections.terminals, sections.agents))
        assertEquals(listOf("new-approval", "old-approval", "answer", "failed"), sections.attention.map { it.sessionId })
        assertEquals(listOf("idle", "working"), sections.running.map { it.sessionId })
        assertEquals(listOf("done"), sections.finished.map { it.sessionId })
    }

    @Test
    fun namesFallBackToIds() {
        val overview = Overview(emptyList(), emptyList(), 0u)
        assertEquals("web", overview.projectName("web"))
        assertEquals("dev", overview.tabTitle("web", "dev"))
    }
}
