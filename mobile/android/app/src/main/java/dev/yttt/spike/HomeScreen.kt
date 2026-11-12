package dev.yttt.spike

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.TextButton
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import uniffi.yttt_mobile_core.AgentState
import uniffi.yttt_mobile_core.AgentSummary
import uniffi.yttt_mobile_core.Overview
import uniffi.yttt_mobile_core.TerminalInfo

/** The home screen: counts, then what needs the user, what runs and what ended. */
@Composable
fun HomeScreen(
    hostLabel: String,
    status: String,
    overview: Overview,
    terminals: List<TerminalInfo>,
    onAgent: (AgentSummary) -> Unit,
    onTerminal: (TerminalInfo) -> Unit,
    onBattery: () -> Unit,
    onMenu: () -> Unit,
) {
    val sections = HomeSections(overview)
    var showFinished by remember { mutableStateOf(false) }
    var showTerminals by remember { mutableStateOf(false) }
    var showLicenses by remember { mutableStateOf(false) }
    if (showLicenses) {
        AlertDialog(
            onDismissRequest = { showLicenses = false },
            title = { Text("开源许可") },
            text = {
                Text(
                    "sora-editor — LGPL-2.1，未修改，作为独立库使用\n" +
                        "ConnectBot termlib — Apache-2.0\n" +
                        "JNA — Apache-2.0 / LGPL-2.1\n" +
                        "Jetpack Compose、AndroidX — Apache-2.0",
                )
            },
            confirmButton = { TextButton(onClick = { showLicenses = false }) { Text("好") } },
        )
    }
    LazyColumn(Modifier.fillMaxWidth()) {
        item {
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(4.dp)) {
                IconButton(onClick = onMenu) { Text("☰", style = MaterialTheme.typography.titleLarge) }
                Text("总控", style = MaterialTheme.typography.titleLarge)
            }
            Text(hostLabel, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.padding(horizontal = 16.dp))
            if (status.isNotEmpty()) Text(status, Modifier.padding(horizontal = 16.dp))
            Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.SpaceEvenly) {
                Count(sections.projects, "项目")
                Count(sections.tabs, "Tab")
                Count(sections.terminals, "终端")
                Count(sections.agents, "Agent")
            }
            HorizontalDivider()
        }
        agentSection("需要处理", sections.attention, overview, onAgent)
        agentSection("运行中", sections.running, overview, onAgent)
        if (sections.finished.isNotEmpty()) {
            item {
                ListItem(
                    headlineContent = { Text("已结束（${sections.finished.size}）") },
                    trailingContent = { Text(if (showFinished) "收起" else "展开") },
                    modifier = Modifier.clickable { showFinished = !showFinished },
                )
            }
            if (showFinished) agentRows(sections.finished, overview, onAgent)
        }
        item {
            ListItem(
                headlineContent = { Text("后台提醒") },
                supportingContent = { Text("关闭本 App 的电池优化，后台更不容易被系统断开") },
                modifier = Modifier.clickable(onClick = onBattery),
            )
        }
        item {
            ListItem(
                headlineContent = { Text("开源许可") },
                modifier = Modifier.clickable { showLicenses = true },
            )
        }
        item {
            ListItem(
                headlineContent = { Text("所有终端（${terminals.size}）") },
                trailingContent = { Text(if (showTerminals) "收起" else "展开") },
                modifier = Modifier.clickable { showTerminals = !showTerminals },
            )
        }
        if (showTerminals) {
            items(terminals, key = { "terminal-" + it.sessionId }) { info ->
                ListItem(
                    headlineContent = { Text(info.sessionId, fontFamily = FontFamily.Monospace) },
                    supportingContent = { Text("${info.cols}×${info.rows} · ${info.projectId}") },
                    modifier = Modifier.clickable { onTerminal(info) },
                )
            }
        }
    }
}

private fun LazyListScope.agentSection(
    title: String,
    agents: List<AgentSummary>,
    overview: Overview,
    onAgent: (AgentSummary) -> Unit,
) {
    if (agents.isEmpty()) return
    item {
        Text(
            title,
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
        )
    }
    agentRows(agents, overview, onAgent)
}

private fun LazyListScope.agentRows(
    agents: List<AgentSummary>,
    overview: Overview,
    onAgent: (AgentSummary) -> Unit,
) {
    items(agents, key = { "agent-" + it.sessionId }) { agent ->
        val urgent = agent.state == AgentState.WAITING || agent.state == AgentState.FAILED
        ListItem(
            headlineContent = {
                Text("${overview.projectName(agent.projectId)} · ${overview.tabTitle(agent.projectId, agent.tabId)}")
            },
            supportingContent = {
                Column {
                    (agent.task ?: agent.waitingMessage)?.let {
                        Text(it, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    }
                    Text(
                        listOfNotNull(agent.provider, agent.elapsed(), agent.currentAction).joinToString(" · "),
                        style = MaterialTheme.typography.bodySmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            },
            trailingContent = {
                Text(
                    agent.stateLabel,
                    fontWeight = FontWeight.Bold,
                    color = if (urgent) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            },
            modifier = Modifier.clickable { onAgent(agent) },
        )
    }
}

@Composable
private fun Count(value: Int, label: String) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Text("$value", style = MaterialTheme.typography.headlineSmall)
        Text(label, style = MaterialTheme.typography.labelSmall)
    }
}
