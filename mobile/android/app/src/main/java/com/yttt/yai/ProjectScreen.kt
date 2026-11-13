package com.yttt.yai

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.outlined.Computer
import androidx.compose.material.icons.outlined.Dashboard
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material.icons.outlined.VerticalSplit
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.PrimaryScrollableTabRow
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import uniffi.yttt_mobile_core.HostSession
import uniffi.yttt_mobile_core.Overview
import uniffi.yttt_mobile_core.PaneSummary
import uniffi.yttt_mobile_core.TerminalInfo

/**
 * A project: the title switches project in place; below it the terminal
 * tabs, each with its Agent's logo and a dot while one waits; the pane's
 * terminal; and, when the tab has several panes, a thin switcher at the
 * bottom, above the key bar while typing.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ProjectScreen(
    session: HostSession,
    link: Link,
    overview: Overview,
    terminals: List<TerminalInfo>,
    initial: Place,
    autoType: String?,
    filesStates: HashMap<String, FilesState>,
    onBack: () -> Unit,
    onHome: () -> Unit,
    onOpenFile: (projectId: String, path: String) -> Unit,
) {
    var place by remember(initial) { mutableStateOf(initial) }
    var switching by remember { mutableStateOf(false) }
    var showFiles by remember { mutableStateOf(false) }
    val project = overview.projects.firstOrNull { it.projectId == place.projectId }
    val tabs = project?.tabs.orEmpty()
    val tab = tabs.firstOrNull { it.tabId == place.tabId }
    val panes = tab?.panes.orEmpty()
    val pane = panes.firstOrNull { it.paneId == place.paneId } ?: panes.firstOrNull()
    val marks = overview.tabMarks(place.projectId)
    val info = pane?.let { candidate -> terminals.firstOrNull { it.sessionId == candidate.sessionId } }
    val control = remember(info?.sessionId) { TerminalControl() }

    Scaffold(
        contentWindowInsets = WindowInsets(0),
        topBar = {
            Column {
                TopAppBar(
                    navigationIcon = {
                        IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "返回") }
                    },
                    title = {
                        Box {
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                modifier = Modifier.clip(RoundedCornerShape(8.dp)).clickable { switching = true }
                                    .padding(vertical = 4.dp),
                            ) {
                                Text(project?.name ?: place.projectId, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Icon(Icons.Filled.ArrowDropDown, "换项目")
                            }
                            ProjectSwitcher(
                                expanded = switching,
                                overview = overview,
                                current = place.projectId,
                                onDismiss = { switching = false },
                                onProject = { id -> switching = false; place = overview.initialPlace(id) },
                                onHome = { switching = false; onHome() },
                            )
                        }
                    },
                    actions = {
                        if (control.controlling) {
                            TextButton(onClick = { control.giveBack() }) {
                                Icon(Icons.Outlined.Computer, null, Modifier.size(18.dp))
                                Spacer(Modifier.width(6.dp))
                                Text("交还电脑")
                            }
                        }
                        IconButton(onClick = { showFiles = true }) { Icon(Icons.Outlined.FolderOpen, "文件与会话") }
                    },
                )
                LinkBanner(link)
                if (tabs.isNotEmpty()) {
                    PrimaryScrollableTabRow(selectedTabIndex = tabs.indexOf(tab).coerceAtLeast(0), edgePadding = 8.dp) {
                        for (candidate in tabs) {
                            val mark = marks[candidate.tabId]
                            Tab(
                                selected = candidate.tabId == tab?.tabId,
                                onClick = { place = overview.placeIn(place.projectId, candidate.tabId) },
                                text = {
                                    BadgedBox(badge = { if (mark?.waiting == true) Badge() }) {
                                        Text(candidate.title, maxLines = 1)
                                    }
                                },
                                icon = {
                                    if (mark?.provider != null) AgentLogo(mark.provider, 18.dp)
                                    else Icon(Icons.Outlined.Terminal, null, Modifier.size(18.dp))
                                },
                            )
                        }
                    }
                }
            }
        },
    ) { padding ->
        val paneBar: @Composable () -> Unit = {
            if (panes.size > 1 && pane != null) {
                PaneBar(panes, pane, overview) { id -> place = place.copy(paneId = id) }
            }
        }
        Box(Modifier.padding(top = padding.calculateTopPadding()).fillMaxSize().background(Color.Black)) {
            if (info != null) {
                // A fresh session makes the pane again.
                key(session, info.sessionId) {
                    val agent = overview.agents.firstOrNull { it.sessionId == info.sessionId }
                    TerminalPane(session, info, tipText(agent), autoType, link, control, above = paneBar)
                }
            } else {
                Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.navigationBars)) {
                    Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                        Text(if (pane == null) "这个项目没有终端" else "这个 pane 没有在运行", color = Color.LightGray)
                    }
                    paneBar()
                }
            }
        }
    }

    if (showFiles) {
        FilesSheet(
            session = session,
            projectId = place.projectId,
            state = filesStates.getOrPut(place.projectId) { FilesState() },
            onDismiss = { showFiles = false },
        ) { path ->
            showFiles = false
            onOpenFile(place.projectId, path)
        }
    }
}

/** The projects, each with its running Agents' logos and waits; picking one opens it here. */
@Composable
private fun ProjectSwitcher(
    expanded: Boolean,
    overview: Overview,
    current: String,
    onDismiss: () -> Unit,
    onProject: (String) -> Unit,
    onHome: () -> Unit,
) {
    DropdownMenu(expanded = expanded, onDismissRequest = onDismiss) {
        for (project in overview.projects) {
            val mark = overview.projectMark(project.projectId)
            DropdownMenuItem(
                leadingIcon = {
                    if (project.projectId == current) Icon(Icons.Filled.Check, "当前")
                    else Icon(Icons.Outlined.Folder, null)
                },
                text = { Text(project.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                trailingIcon = {
                    Row(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically) {
                        for (provider in mark.providers.take(3)) AgentLogo(provider, 16.dp)
                        if (mark.waiting > 0) Badge { Text("${mark.waiting}") }
                    }
                },
                onClick = { onProject(project.projectId) },
            )
        }
        HorizontalDivider()
        DropdownMenuItem(
            leadingIcon = { Icon(Icons.Outlined.Dashboard, null) },
            text = { Text("回到总控") },
            onClick = onHome,
        )
    }
}

/**
 * The tab's panes in a thin bar, tapped or swiped through; each pane with
 * its Agent's logo.
 */
@Composable
private fun PaneBar(panes: List<PaneSummary>, current: PaneSummary, overview: Overview, onPane: (String?) -> Unit) {
    val index = panes.indexOf(current)
    Row(
        Modifier
            .fillMaxWidth()
            .height(28.dp)
            .background(Color(0xFF1D2024))
            .pointerInput(panes, current.paneId) {
                var dragged = 0f
                detectHorizontalDragGestures(
                    onDragStart = { dragged = 0f },
                    onHorizontalDrag = { _, amount -> dragged += amount },
                    onDragEnd = {
                        if (dragged < -48) onPane(step(panes, current.paneId, +1))
                        if (dragged > 48) onPane(step(panes, current.paneId, -1))
                    },
                )
            }
            .padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Icon(Icons.Outlined.VerticalSplit, "pane", Modifier.size(14.dp), tint = Color(0xFF8E9099))
        for (candidate in panes) {
            val selected = candidate.paneId == current.paneId
            val provider = overview.agents.firstOrNull { it.sessionId == candidate.sessionId }?.provider
            Row(
                Modifier
                    .clip(RoundedCornerShape(6.dp))
                    .background(if (selected) Color(0xFF3E4759) else Color.Transparent)
                    .clickable { onPane(candidate.paneId) }
                    .padding(horizontal = 10.dp, vertical = 2.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                if (provider != null) AgentLogo(provider, 12.dp)
                Text(
                    candidate.title,
                    fontSize = 11.sp,
                    maxLines = 1,
                    color = if (selected) Color(0xFFDAE2F9) else Color(0xFFC4C6D0),
                    fontWeight = if (selected) FontWeight.Medium else FontWeight.Normal,
                )
            }
        }
        Spacer(Modifier.weight(1f))
        Text("${index + 1}/${panes.size}", fontSize = 11.sp, color = Color(0xFF8E9099))
    }
}

/** A terminal on its own, when it runs in no pane. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TerminalScreen(session: HostSession, link: Link, overview: Overview, info: TerminalInfo, onBack: () -> Unit) {
    val control = remember(info.sessionId) { TerminalControl() }
    Scaffold(
        contentWindowInsets = WindowInsets(0),
        topBar = {
            Column {
                TopAppBar(
                    navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "返回") } },
                    title = { Text(info.sessionId, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.titleMedium) },
                    actions = {
                        if (control.controlling) TextButton(onClick = { control.giveBack() }) { Text("交还电脑") }
                    },
                )
                LinkBanner(link)
            }
        },
    ) { padding ->
        Box(Modifier.padding(top = padding.calculateTopPadding()).fillMaxSize()) {
            val agent = overview.agents.firstOrNull { it.sessionId == info.sessionId }
            TerminalPane(session, info, tipText(agent), null, link, control)
        }
    }
}
