package dev.yttt.spike

import uniffi.yttt_mobile_core.AgentState
import uniffi.yttt_mobile_core.AgentSummary
import uniffi.yttt_mobile_core.Overview
import uniffi.yttt_mobile_core.PaneSummary
import uniffi.yttt_mobile_core.WaitingKind

/** A project's counts in the left drawer. */
data class Badges(val waiting: Int, val failed: Int, val running: Int)

/** Where the project page shows: a project, and in it a tab and a pane. */
data class Place(val projectId: String, val tabId: String?, val paneId: String?)

fun Overview.badges(): Map<String, Badges> =
    agents.groupBy { it.projectId }.mapValues { (_, agents) ->
        Badges(
            waiting = agents.count { it.state == AgentState.WAITING },
            failed = agents.count { it.state == AgentState.FAILED },
            running = agents.count {
                it.state == AgentState.WORKING || it.state == AgentState.STARTING || it.state == AgentState.IDLE
            },
        )
    }

/** A project as the desktop shows it: its selected tab, that tab's focused pane. */
fun Overview.initialPlace(projectId: String): Place {
    val project = projects.firstOrNull { it.projectId == projectId } ?: return Place(projectId, null, null)
    val tab = project.tabs.firstOrNull { it.tabId == project.selectedTabId } ?: project.tabs.firstOrNull()
        ?: return Place(projectId, null, null)
    return placeIn(projectId, tab.tabId)
}

/** A tab opens on its focused pane, or its first. */
fun Overview.placeIn(projectId: String, tabId: String): Place {
    val tab = projects.firstOrNull { it.projectId == projectId }?.tabs?.firstOrNull { it.tabId == tabId }
    val pane = tab?.panes?.firstOrNull { it.paneId == tab.focusedPaneId } ?: tab?.panes?.firstOrNull()
    return Place(projectId, tab?.tabId, pane?.paneId)
}

/** The pane running terminal `sessionId`, as for an Agent or a notification. */
fun Overview.placeOf(sessionId: String): Place? {
    for (project in projects) {
        for (tab in project.tabs) {
            tab.panes.firstOrNull { it.sessionId == sessionId }?.let {
                return Place(project.projectId, tab.tabId, it.paneId)
            }
        }
    }
    return null
}

/** The pane `by` steps from `current` within `panes`, staying at the ends. */
fun step(panes: List<PaneSummary>, current: String?, by: Int): String? {
    if (panes.isEmpty()) return null
    val index = panes.indexOfFirst { it.paneId == current }.coerceAtLeast(0)
    return panes[(index + by).coerceIn(0, panes.lastIndex)].paneId
}

/** The take-control tip, saying what the pane's Agent waits for. */
fun tipText(agent: AgentSummary?): String =
    if (agent?.state != AgentState.WAITING) "接管终端"
    else when (agent.waiting) {
        WaitingKind.APPROVAL -> "等批准，接管后回复"
        WaitingKind.USER_INPUT -> "等回答，接管后回复"
        else -> "等待中，接管后处理"
    }
