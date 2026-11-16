package com.yttt.yai

import android.content.Context
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Bolt
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Cloud
import androidx.compose.material.icons.filled.PanTool
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.LaptopMac
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.SmartToy
import androidx.compose.material.icons.outlined.Tab
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Card
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.yttt_mobile_core.AgentSummary
import uniffi.yttt_mobile_core.HostSession
import uniffi.yttt_mobile_core.LinkPath
import uniffi.yttt_mobile_core.Overview

/** The paired Hosts, for the menu on the overview's Host chip. */
class HostMenu(
    val names: List<Pair<String, String>>,
    val current: String?,
    val onSwitch: (String) -> Unit,
    val onAdd: () -> Unit,
    val onManage: () -> Unit,
)

/**
 * The overview: the Host and its link, counts, what runs right now, then
 * every Agent under its project, each project foldable.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun OverviewScreen(
    session: HostSession?,
    hostLabel: String,
    hosts: HostMenu,
    status: String,
    overview: Overview,
    onAgent: (AgentSummary) -> Unit,
    onRefresh: suspend () -> Unit,
    banner: @Composable () -> Unit = {},
) {
    val sections = HomeSections(overview)
    val context = LocalContext.current
    val prefs = remember { context.getSharedPreferences(OVERVIEW_PREFS, Context.MODE_PRIVATE) }
    // Project ids whose Agent group is folded; kept across launches.
    var collapsed by remember { mutableStateOf(prefs.getStringSet(COLLAPSED_PROJECTS, emptySet()).orEmpty().toSet()) }
    var refreshing by remember { mutableStateOf(false) }
    var path by remember { mutableStateOf(LinkPath.UNKNOWN) }
    val scope = rememberCoroutineScope()
    val scroll = TopAppBarDefaults.enterAlwaysScrollBehavior()
    // Direct or through the relay, as it changes.
    LaunchedEffect(session) {
        while (session != null) {
            path = runCatching { session.path() }.getOrDefault(LinkPath.UNKNOWN)
            delay(3000)
        }
    }

    Scaffold(
        contentWindowInsets = WindowInsets(0),
        modifier = Modifier.nestedScroll(scroll.nestedScrollConnection),
        topBar = {
            Column {
                TopAppBar(title = { Text("总控") }, scrollBehavior = scroll)
                banner()
            }
        },
    ) { padding ->
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = { scope.launch { refreshing = true; onRefresh(); refreshing = false } },
            modifier = Modifier.padding(padding).fillMaxSize(),
        ) {
            LazyColumn(Modifier.fillMaxSize()) {
                item { HostChip(hostLabel, hosts, linkLabel(session, status, path), session != null && status.isEmpty(), path) }
                item { Counts(sections) }
                if (overview.agents.isEmpty()) {
                    item { EmptyAgents(session != null) }
                }
                runningCard(sections.running, overview, onAgent)
                for (group in sections.groups) {
                    projectCard(group, group.projectId in collapsed, overview, onAgent) {
                        collapsed = if (group.projectId in collapsed) collapsed - group.projectId else collapsed + group.projectId
                        prefs.edit().putStringSet(COLLAPSED_PROJECTS, collapsed).apply()
                    }
                }
                item { Spacer(Modifier.height(16.dp)) }
            }
        }
    }
}

private fun linkLabel(session: HostSession?, status: String, path: LinkPath): String = when {
    status.isNotEmpty() -> status
    session == null -> "未连接"
    path == LinkPath.RELAY -> "已连接 · 经中继"
    path == LinkPath.DIRECT -> "已连接 · 直连"
    else -> "已连接"
}

@Composable
private fun HostChip(hostLabel: String, hosts: HostMenu, label: String, connected: Boolean, path: LinkPath) {
    var open by remember { mutableStateOf(false) }
    Box(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
        AssistChip(
            onClick = { open = true },
            label = { Text("$hostLabel · $label", maxLines = 1, overflow = TextOverflow.Ellipsis) },
            leadingIcon = { Icon(Icons.Outlined.LaptopMac, null, Modifier.size(18.dp)) },
            trailingIcon = {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (connected) {
                        Icon(if (path == LinkPath.RELAY) Icons.Filled.Cloud else Icons.Filled.Bolt, null, Modifier.size(14.dp))
                    }
                    Icon(Icons.Filled.ArrowDropDown, "切换 Host", Modifier.size(18.dp))
                }
            },
        )
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            for ((id, name) in hosts.names) {
                DropdownMenuItem(
                    leadingIcon = { if (id == hosts.current) Icon(Icons.Filled.Check, "当前") else Icon(Icons.Outlined.LaptopMac, null) },
                    text = { Text(name) },
                    onClick = { open = false; hosts.onSwitch(id) },
                )
            }
            HorizontalDivider()
            DropdownMenuItem(leadingIcon = { Icon(Icons.Outlined.Add, null) }, text = { Text("添加 Host") },
                onClick = { open = false; hosts.onAdd() })
            DropdownMenuItem(leadingIcon = { Icon(Icons.Outlined.Settings, null) }, text = { Text("管理 Host") },
                onClick = { open = false; hosts.onManage() })
        }
    }
}

@Composable
private fun Counts(sections: HomeSections) {
    Card(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
        Row(Modifier.fillMaxWidth().padding(vertical = 14.dp), horizontalArrangement = Arrangement.SpaceEvenly) {
            Count(Icons.Outlined.Folder, sections.projects, "项目")
            Count(Icons.Outlined.Tab, sections.tabs, "Tab")
            Count(Icons.Outlined.Terminal, sections.terminals, "终端")
            Count(Icons.Outlined.SmartToy, sections.agents, "Agent")
        }
    }
}

@Composable
private fun Count(icon: ImageVector, value: Int, label: String) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Icon(icon, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary)
        Text("$value", style = MaterialTheme.typography.headlineSmall)
        Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun EmptyAgents(connected: Boolean) {
    Column(
        Modifier.fillMaxWidth().padding(vertical = 56.dp, horizontal = 32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Icon(Icons.Outlined.SmartToy, null, Modifier.size(48.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(if (connected) "还没有 Agent" else "还没连上 Host", style = MaterialTheme.typography.titleMedium)
        Text(
            if (connected) "在电脑上启动一个 Agent，它会出现在这里。" else "连上以后，这里会列出电脑上的 Agent。",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

private const val OVERVIEW_PREFS = "overview"
private const val COLLAPSED_PROJECTS = "collapsedProjects"

/** The working and starting Agents as a card; its title is blue and shimmers. */
private fun LazyListScope.runningCard(agents: List<AgentSummary>, overview: Overview, onAgent: (AgentSummary) -> Unit) {
    if (agents.isEmpty()) return
    item(key = "card-running") {
        Card(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).animateContentSize()) {
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Box(
                    Modifier.size(28.dp).background(runningBlue, RoundedCornerShape(7.dp)),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(Icons.Filled.Bolt, null, Modifier.size(16.dp), tint = Color.White)
                }
                Text(
                    "执行中",
                    style = MaterialTheme.typography.titleMedium,
                    color = runningBlue,
                    modifier = Modifier.weight(1f, fill = false).shimmer(true),
                )
                Spacer(Modifier.weight(1f))
                Text("${agents.size}", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            for (agent in agents) {
                HorizontalDivider(Modifier.padding(start = 16.dp))
                AgentRow(agent, overview, onAgent, showProject = true)
            }
        }
    }
}

/**
 * A project's Agents as a card, as the desktop sidebar groups them: the
 * title row folds it and shows its waits and runs.
 */
private fun LazyListScope.projectCard(
    group: HomeSections.ProjectGroup,
    collapsed: Boolean,
    overview: Overview,
    onAgent: (AgentSummary) -> Unit,
    onToggle: () -> Unit,
) {
    item(key = "project-${group.projectId}") {
        val waiting = group.agents.count { HomeSections.attentionRank(it) != null }
        val running = group.agents.count { HomeSections.isRunning(it) }
        val chevron by animateFloatAsState(if (collapsed) 0f else 90f, label = "chevron")
        Card(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).animateContentSize()) {
            Row(
                Modifier.fillMaxWidth().clickable(onClick = onToggle).padding(horizontal = 16.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Box(
                    Modifier.size(28.dp).background(MaterialTheme.colorScheme.primary, RoundedCornerShape(7.dp)),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(Icons.Filled.Folder, null, Modifier.size(16.dp), tint = Color.White)
                }
                Text(
                    group.name,
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                if (waiting > 0) Mark(Icons.Filled.PanTool, waiting, MaterialTheme.colorScheme.error)
                if (running > 0) Mark(Icons.Filled.Bolt, running, runningBlue)
                Text("${group.agents.size}", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Icon(
                    Icons.AutoMirrored.Filled.KeyboardArrowRight,
                    if (collapsed) "展开" else "收起",
                    Modifier.size(20.dp).rotate(chevron),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (!collapsed) {
                for (agent in group.agents) {
                    HorizontalDivider(Modifier.padding(start = 16.dp))
                    AgentRow(agent, overview, onAgent, showProject = false)
                }
            }
        }
    }
}

@Composable
private fun Mark(icon: ImageVector, value: Int, color: Color) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(2.dp)) {
        Icon(icon, null, Modifier.size(14.dp), tint = color)
        Text("$value", style = MaterialTheme.typography.labelLarge, color = color)
    }
}

@Composable
private fun AgentRow(agent: AgentSummary, overview: Overview, onAgent: (AgentSummary) -> Unit, showProject: Boolean) {
    ListItem(
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        leadingContent = { AgentTile(agent.provider) },
        headlineContent = {
            val tab = overview.tabTitle(agent.projectId, agent.tabId)
            Text(
                if (showProject) "${overview.projectName(agent.projectId)} · $tab" else tab,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        },
        supportingContent = {
            Column {
                (agent.waitingMessage ?: agent.task)?.let { Text(it, maxLines = 2, overflow = TextOverflow.Ellipsis) }
                Text(
                    listOfNotNull(agent.elapsed(), agent.currentAction).joinToString(" · "),
                    style = MaterialTheme.typography.bodySmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        },
        trailingContent = { AgentStateLabel(agent) },
        modifier = Modifier.clickable { onAgent(agent) },
    )
}
