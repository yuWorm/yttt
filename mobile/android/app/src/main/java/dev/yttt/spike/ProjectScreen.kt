package dev.yttt.spike

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Badge
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.NavigationDrawerItem
import androidx.compose.material3.ScrollableTabRow
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import uniffi.yttt_mobile_core.HostSession
import uniffi.yttt_mobile_core.Overview
import uniffi.yttt_mobile_core.TerminalInfo

/** The left drawer: the overview, the projects with their counts, the Host. */
@Composable
fun LeftDrawer(
    hostLabel: String,
    overview: Overview,
    current: String?,
    onHome: () -> Unit,
    onProject: (String) -> Unit,
    onForget: () -> Unit,
) {
    val badges = overview.badges()
    ModalDrawerSheet {
        Text(hostLabel, style = MaterialTheme.typography.titleMedium, maxLines = 1,
            overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(16.dp))
        NavigationDrawerItem(
            label = { Text("总控") },
            selected = current == null,
            onClick = onHome,
            modifier = Modifier.padding(horizontal = 12.dp),
        )
        HorizontalDivider(Modifier.padding(vertical = 8.dp))
        Text("项目", style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(start = 28.dp, bottom = 4.dp))
        for (project in overview.projects) {
            val badge = badges[project.projectId]
            NavigationDrawerItem(
                label = { Text(project.name) },
                selected = current == project.projectId,
                onClick = { onProject(project.projectId) },
                badge = {
                    Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        badge?.waiting?.takeIf { it > 0 }?.let {
                            Badge(containerColor = MaterialTheme.colorScheme.error) { Text("等 $it") }
                        }
                        badge?.failed?.takeIf { it > 0 }?.let {
                            Badge(containerColor = MaterialTheme.colorScheme.errorContainer) { Text("败 $it") }
                        }
                        badge?.running?.takeIf { it > 0 }?.let { Badge { Text("运行 $it") } }
                    }
                },
                modifier = Modifier.padding(horizontal = 12.dp),
            )
        }
        HorizontalDivider(Modifier.padding(vertical = 8.dp))
        NavigationDrawerItem(
            label = { Text("忘记这台 Host") },
            selected = false,
            onClick = onForget,
            modifier = Modifier.padding(horizontal = 12.dp),
        )
    }
}

/**
 * A project: its terminal tabs, a switcher when the tab has several panes,
 * and the pane's terminal.
 */
@Composable
fun ProjectScreen(
    session: HostSession,
    link: Link,
    overview: Overview,
    terminals: List<TerminalInfo>,
    place: Place,
    autoType: String?,
    onPlace: (Place) -> Unit,
    onMenu: () -> Unit,
) {
    val project = overview.projects.firstOrNull { it.projectId == place.projectId }
    val tabs = project?.tabs.orEmpty()
    val tab = tabs.firstOrNull { it.tabId == place.tabId }
    val panes = tab?.panes.orEmpty()
    val pane = panes.firstOrNull { it.paneId == place.paneId } ?: panes.firstOrNull()
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onMenu) { Text("☰", style = MaterialTheme.typography.titleLarge) }
            Text(project?.name ?: place.projectId, style = MaterialTheme.typography.titleLarge)
        }
        if (tabs.isNotEmpty()) {
            ScrollableTabRow(selectedTabIndex = tabs.indexOf(tab).coerceAtLeast(0), edgePadding = 8.dp) {
                for (candidate in tabs) {
                    Tab(
                        selected = candidate.tabId == tab?.tabId,
                        onClick = { onPlace(overview.placeIn(place.projectId, candidate.tabId)) },
                        text = { Text(candidate.title) },
                    )
                }
            }
        }
        if (panes.size > 1 && pane != null) {
            val index = panes.indexOf(pane)
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = { onPlace(place.copy(paneId = step(panes, pane.paneId, -1))) }, enabled = index > 0) {
                    Text("◀")
                }
                Text("${index + 1}/${panes.size} · ${pane.title}", Modifier.weight(1f), maxLines = 1,
                    overflow = TextOverflow.Ellipsis)
                TextButton(onClick = { onPlace(place.copy(paneId = step(panes, pane.paneId, +1))) },
                    enabled = index < panes.lastIndex) {
                    Text("▶")
                }
            }
        }
        val info = pane?.let { candidate -> terminals.firstOrNull { it.sessionId == candidate.sessionId } }
        Box(Modifier.fillMaxSize()) {
            when {
                info != null -> key(info.sessionId) {
                    val agent = overview.agents.firstOrNull { it.sessionId == info.sessionId }
                    TerminalPane(session, info, tipText(agent), autoType, link, onBack = null)
                }
                pane != null -> Text("这个 pane 没有在运行", Modifier.align(Alignment.Center))
                else -> Text("这个项目没有终端", Modifier.align(Alignment.Center))
            }
        }
    }
}
