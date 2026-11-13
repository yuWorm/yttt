package dev.yttt.spike

import uniffi.yttt_mobile_core.AgentState
import uniffi.yttt_mobile_core.Overview
import uniffi.yttt_mobile_core.WaitingKind

/** The ongoing notification of the foreground service. */
const val LINK_NOTIFICATION_ID = 1

sealed interface NotificationChange {
    /** `quiet`: posted while the app is on screen, to the shade without a banner. */
    data class Post(val sessionId: String, val title: String, val text: String, val quiet: Boolean = false) :
        NotificationChange
    data class Cancel(val sessionId: String) : NotificationChange
}

/**
 * Decides, from each new overview, which "Agent is waiting" notifications to
 * post and which to take back. One wait is one Agent in `WAITING` since one
 * `stateStartedAtMs`: it is notified at most once, not at all if the user is
 * looking at that terminal when it starts (they have seen it), and the
 * notification goes when the wait ends. While the app is on screen it is
 * posted quietly: the home screen already shows it, and a banner would cover
 * the top bar. Failures are not notified. Not
 * thread-safe: call from one coroutine.
 */
class WaitNotifier {
    /** Each waiting Agent's current wait, and whether it has a notification. */
    private val waits = HashMap<String, Wait>()

    private data class Wait(val since: ULong, val posted: Boolean)

    fun update(overview: Overview, viewing: String?, foreground: Boolean = false): List<NotificationChange> {
        val changes = mutableListOf<NotificationChange>()
        val waiting = overview.agents.filter { it.state == AgentState.WAITING }.associateBy { it.sessionId }
        for ((sessionId, wait) in waits.toMap()) {
            val agent = waiting[sessionId]
            if (agent == null || agent.stateStartedAtMs != wait.since) {
                waits.remove(sessionId)
                if (wait.posted) changes += NotificationChange.Cancel(sessionId)
            }
        }
        for ((sessionId, agent) in waiting) {
            if (sessionId in waits) continue
            val seen = sessionId == viewing
            waits[sessionId] = Wait(agent.stateStartedAtMs, posted = !seen)
            if (seen) continue
            val what = when (agent.waiting) {
                WaitingKind.APPROVAL -> "等批准"
                WaitingKind.USER_INPUT -> "等回答"
                else -> "等待"
            }
            val detail = agent.waitingMessage ?: agent.task
            changes += NotificationChange.Post(
                sessionId = sessionId,
                title = "${overview.projectName(agent.projectId)} · ${overview.tabTitle(agent.projectId, agent.tabId)}",
                text = if (detail != null) "$what：$detail" else what,
                quiet = foreground,
            )
        }
        return changes
    }

    companion object {
        /** Stable per Agent, so a newer wait replaces an older notification. */
        fun notificationId(sessionId: String): Int {
            val hash = sessionId.hashCode()
            return if (hash == LINK_NOTIFICATION_ID) hash + 1 else hash
        }
    }
}
