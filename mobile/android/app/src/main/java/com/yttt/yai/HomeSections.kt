package com.yttt.yai

import uniffi.yttt_mobile_core.AgentState
import uniffi.yttt_mobile_core.AgentSummary
import uniffi.yttt_mobile_core.Overview
import uniffi.yttt_mobile_core.WaitingKind

/**
 * The home screen's groups, counted and sorted natively from the core's
 * overview: what runs right now, then every Agent under its project, as the
 * desktop sidebar groups them.
 */
class HomeSections(overview: Overview) {
    /** A project's Agents, most urgent first. */
    class ProjectGroup(val projectId: String, val name: String, val agents: List<AgentSummary>)

    val projects = overview.projects.size
    val tabs = overview.projects.sumOf { it.tabs.size }
    val terminals = overview.terminals.toInt()
    val agents = overview.agents.size

    private val urgentFirst = compareBy<AgentSummary> { rank(it) }.thenByDescending { it.stateStartedAtMs }

    /** Waiting for approval, then an answer, then anything else, then failed; recent first. */
    val attention: List<AgentSummary> = overview.agents.filter { attentionRank(it) != null }.sortedWith(urgentFirst)

    /** Working or starting; an idle Agent is not running. */
    val running: List<AgentSummary> = overview.agents.filter { isRunning(it) }.sortedWith(urgentFirst)

    /**
     * Projects in the Host's order, then Agents of projects no longer
     * listed; projects without Agents are left out.
     */
    val groups: List<ProjectGroup> = run {
        val byProject = overview.agents.groupBy { it.projectId }
        val listed = overview.projects.map { it.projectId to it.name }
        val known = listed.map { it.first }.toSet()
        val unlisted = overview.agents.map { it.projectId }.filter { it !in known }.distinct().map { it to it }
        (listed + unlisted).mapNotNull { (id, name) ->
            byProject[id]?.let { ProjectGroup(id, name, it.sortedWith(urgentFirst)) }
        }
    }

    companion object {
        fun isRunning(agent: AgentSummary): Boolean = agent.state == AgentState.WORKING || agent.state == AgentState.STARTING

        /** Needs the user (0–3), runs (4), idles (5), ended (6). */
        fun rank(agent: AgentSummary): Int = attentionRank(agent) ?: when (agent.state) {
            AgentState.WORKING, AgentState.STARTING -> 4
            AgentState.IDLE -> 5
            else -> 6
        }

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
