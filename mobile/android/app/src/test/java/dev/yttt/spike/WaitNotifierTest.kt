package dev.yttt.spike

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.yttt_mobile_core.AgentState
import uniffi.yttt_mobile_core.AgentSummary
import uniffi.yttt_mobile_core.Overview
import uniffi.yttt_mobile_core.ProjectSummary
import uniffi.yttt_mobile_core.TabSummary
import uniffi.yttt_mobile_core.WaitingKind

class WaitNotifierTest {
    private fun agent(
        id: String,
        state: AgentState,
        waiting: WaitingKind? = null,
        since: Long = 1,
        message: String? = null,
        task: String? = "Fix the build",
    ) = AgentSummary(
        sessionId = id, projectId = "web", tabId = "build", paneId = id, provider = "codex",
        state = state, waiting = waiting, waitingMessage = message, task = task,
        currentAction = null, stateStartedAtMs = since.toULong(), updatedAtMs = since.toULong(),
    )

    private fun overview(vararg agents: AgentSummary) = Overview(
        projects = listOf(ProjectSummary("web", "Website", listOf(TabSummary("build", "Build", 1u)))),
        agents = agents.toList(),
        terminals = 1u,
    )

    @Test
    fun anAgentThatStartsWaitingIsNotifiedOnce() {
        val notifier = WaitNotifier()
        assertEquals(emptyList<NotificationChange>(), notifier.update(overview(agent("a", AgentState.WORKING)), null))
        val waiting = overview(agent("a", AgentState.WAITING, WaitingKind.APPROVAL, since = 5, message = "Run npm install?"))
        val posted = notifier.update(waiting, null)
        assertEquals(1, posted.size)
        val post = posted.single() as NotificationChange.Post
        assertEquals("a", post.sessionId)
        assertEquals("Website · Build", post.title)
        assertEquals("等批准：Run npm install?", post.text)
        assertEquals(emptyList<NotificationChange>(), notifier.update(waiting, null))
    }

    @Test
    fun theTaskStandsInForAMissingMessageAndTheReasonPicksTheWords() {
        val notifier = WaitNotifier()
        val post = notifier.update(overview(agent("a", AgentState.WAITING, WaitingKind.USER_INPUT)), null)
            .single() as NotificationChange.Post
        assertEquals("等回答：Fix the build", post.text)
    }

    @Test
    fun theNotificationGoesWhenTheWaitEnds() {
        val notifier = WaitNotifier()
        notifier.update(overview(agent("a", AgentState.WAITING, WaitingKind.APPROVAL, since = 5)), null)
        assertEquals(
            listOf(NotificationChange.Cancel("a")),
            notifier.update(overview(agent("a", AgentState.WORKING, since = 9)), null),
        )
        // And when the Agent is gone.
        notifier.update(overview(agent("a", AgentState.WAITING, WaitingKind.APPROVAL, since = 12)), null)
        assertEquals(listOf(NotificationChange.Cancel("a")), notifier.update(overview(), null))
    }

    @Test
    fun noNotificationWhileTheUserLooksAtThatTerminalNorAfterwardsForTheSameWait() {
        val notifier = WaitNotifier()
        val waiting = overview(agent("a", AgentState.WAITING, WaitingKind.APPROVAL, since = 5))
        assertEquals(emptyList<NotificationChange>(), notifier.update(waiting, viewing = "a"))
        assertEquals(emptyList<NotificationChange>(), notifier.update(waiting, viewing = null))
        // A new wait is new news.
        notifier.update(overview(agent("a", AgentState.WORKING, since = 7)), null)
        val again = notifier.update(overview(agent("a", AgentState.WAITING, WaitingKind.APPROVAL, since = 9)), null)
        assertTrue(again.single() is NotificationChange.Post)
    }

    @Test
    fun agentsAreIndependent() {
        val notifier = WaitNotifier()
        notifier.update(overview(agent("a", AgentState.WAITING, WaitingKind.APPROVAL, since = 5)), null)
        val changes = notifier.update(
            overview(
                agent("a", AgentState.WAITING, WaitingKind.APPROVAL, since = 5),
                agent("b", AgentState.WAITING, WaitingKind.EXTERNAL, since = 6),
            ),
            null,
        )
        assertEquals(listOf("b"), changes.map { (it as NotificationChange.Post).sessionId })
        assertEquals("等待：Fix the build", (changes.single() as NotificationChange.Post).text)
        // Failed Agents are not notified.
        assertEquals(
            listOf(NotificationChange.Cancel("a")),
            notifier.update(
                overview(
                    agent("a", AgentState.FAILED, since = 8),
                    agent("b", AgentState.WAITING, WaitingKind.EXTERNAL, since = 6),
                ),
                null,
            ),
        )
    }

    @Test
    fun notificationIdsAreStablePerAgent() {
        assertEquals(WaitNotifier.notificationId("web:build:codex"), WaitNotifier.notificationId("web:build:codex"))
        assertTrue(WaitNotifier.notificationId("a") != WaitNotifier.notificationId("b"))
        assertTrue(WaitNotifier.notificationId("a") != LINK_NOTIFICATION_ID)
    }
}
