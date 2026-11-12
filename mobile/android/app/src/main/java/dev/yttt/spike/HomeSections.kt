package dev.yttt.spike

import uniffi.yttt_mobile_core.AgentState
import uniffi.yttt_mobile_core.AgentSummary
import uniffi.yttt_mobile_core.Overview
import uniffi.yttt_mobile_core.WaitingKind

/**
 * The home screen's groups, counted and sorted natively from the core's
 * overview: what needs the user first, then what runs, then what ended.
 */
class HomeSections(overview: Overview) {
    val projects = overview.projects.size
    val tabs = overview.projects.sumOf { it.tabs.size }
    val terminals = overview.terminals.toInt()
    val agents = overview.agents.size

    /** Waiting for approval, then an answer, then anything else, then failed; recent first. */
    val attention: List<AgentSummary> = overview.agents
        .filter { attentionRank(it) != null }
        .sortedWith(compareBy<AgentSummary> { attentionRank(it) }.thenByDescending { it.stateStartedAtMs })
    val running: List<AgentSummary> = overview.agents
        .filter { it.state in setOf(AgentState.WORKING, AgentState.STARTING, AgentState.IDLE) }
        .sortedByDescending { it.stateStartedAtMs }
    val finished: List<AgentSummary> = overview.agents
        .filter { it.state in setOf(AgentState.COMPLETED, AgentState.INTERRUPTED, AgentState.STALE) }
        .sortedByDescending { it.stateStartedAtMs }

    companion object {
        fun attentionRank(agent: AgentSummary): Int? = when (agent.state) {
            AgentState.WAITING -> when (agent.waiting) {
                WaitingKind.APPROVAL -> 0
                WaitingKind.USER_INPUT -> 1
                else -> 2
            }
            AgentState.FAILED -> 3
            else -> null
        }
    }
}

fun Overview.projectName(id: String): String = projects.firstOrNull { it.projectId == id }?.name ?: id

fun Overview.tabTitle(project: String, tab: String): String =
    projects.firstOrNull { it.projectId == project }?.tabs?.firstOrNull { it.tabId == tab }?.title ?: tab

val AgentSummary.stateLabel: String
    get() = when (state) {
        AgentState.WAITING -> when (waiting) {
            WaitingKind.APPROVAL -> "等批准"
            WaitingKind.USER_INPUT -> "等回答"
            else -> "等待"
        }
        AgentState.FAILED -> "失败"
        AgentState.WORKING -> "工作中"
        AgentState.STARTING -> "启动中"
        AgentState.IDLE -> "空闲"
        AgentState.COMPLETED -> "完成"
        AgentState.INTERRUPTED -> "已中断"
        AgentState.STALE -> "无响应"
    }

/** How long the Agent has been in its state, e.g. "3 分钟". */
fun AgentSummary.elapsed(nowMs: Long = System.currentTimeMillis()): String {
    val seconds = ((nowMs - stateStartedAtMs.toLong()) / 1000).coerceAtLeast(0)
    return when {
        seconds < 60 -> "$seconds 秒"
        seconds < 3600 -> "${seconds / 60} 分钟"
        else -> "${seconds / 3600} 小时"
    }
}
