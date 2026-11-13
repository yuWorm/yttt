package com.yttt.yai

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.outlined.Article
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.FolderOpen
import androidx.compose.material.icons.outlined.Code
import androidx.compose.material.icons.outlined.DataObject
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.History
import androidx.compose.material.icons.outlined.Image
import androidx.compose.material.icons.outlined.Javascript
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.PrimaryTabRow
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import uniffi.yttt_mobile_core.HostSession
import uniffi.yttt_mobile_core.SessionEntry
import java.text.DateFormat
import java.util.Date

/** One project's files and sessions, kept while the sheet is closed so the tree is as the user left it. */
class FilesState {
    val tree = FileTree()
    var version by mutableIntStateOf(0)
    var section by mutableIntStateOf(0)
    var sessions by mutableStateOf<List<SessionEntry>?>(null)
    var error by mutableStateOf<String?>(null)
    var rootRead = false
}

/** A file's icon and its colour, by extension. */
private fun fileIcon(name: String): Pair<ImageVector, Color?> = when (name.substringAfterLast('.', "").lowercase()) {
    "ts", "tsx", "js", "jsx", "mjs" -> Icons.Outlined.Javascript to Color(0xFF3178C6)
    "json", "toml", "yaml", "yml" -> Icons.Outlined.DataObject to Color(0xFFE0A100)
    "md", "txt" -> Icons.AutoMirrored.Outlined.Article to null
    "rs", "kt", "swift", "py", "go", "c", "h", "cpp", "java" -> Icons.Outlined.Code to Color(0xFF7E57C2)
    "sh", "zsh", "bash", "ps1" -> Icons.Outlined.Terminal to Color(0xFF2E7D32)
    "png", "jpg", "jpeg", "gif", "svg", "webp" -> Icons.Outlined.Image to Color(0xFFD81B60)
    else -> if (name.startsWith(".")) Icons.Outlined.Settings to null else Icons.Outlined.Description to null
}

/** The project's files, a directory at a time, and its Agent sessions, in a bottom sheet. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FilesSheet(
    session: HostSession,
    projectId: String,
    state: FilesState,
    onDismiss: () -> Unit,
    onOpenFile: (String) -> Unit,
) {
    val tree = state.tree
    val scope = rememberCoroutineScope()
    val sheet = rememberModalBottomSheetState()

    suspend fun read(directory: String) {
        runCatching { session.listDirectory(projectId, directory) }
            .onSuccess { tree.loaded(directory, it); state.version++ }
            .onFailure { state.error = it.message }
    }

    LaunchedEffect(projectId) {
        if (!state.rootRead) {
            state.rootRead = true
            read("")
        }
    }
    LaunchedEffect(projectId, state.section) {
        if (state.section == 1 && state.sessions == null) {
            state.sessions = runCatching { session.agentSessions(projectId) }
                .onFailure { state.error = it.message }
                .getOrDefault(emptyList())
        }
    }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet) {
        PrimaryTabRow(selectedTabIndex = state.section) {
            Tab(state.section == 0, onClick = { state.section = 0 }, text = { Text("文件") },
                icon = { Icon(Icons.Outlined.Folder, null) })
            Tab(state.section == 1, onClick = { state.section = 1 }, text = { Text("会话") },
                icon = { Icon(Icons.Outlined.History, null) })
        }
        state.error?.let { Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(16.dp)) }
        val itemColors = ListItemDefaults.colors(containerColor = Color.Transparent)
        if (state.section == 0) {
            // Read `version` so the list follows the tree's changes.
            val rows = state.version.let { tree.rows() }
            LazyColumn(Modifier.fillMaxHeight()) {
                items(rows, key = { it.entry.path }) { row ->
                    val entry = row.entry
                    val expanded = entry.isDirectory && tree.isExpanded(entry.path)
                    ListItem(
                        colors = itemColors,
                        leadingContent = {
                            if (entry.isDirectory) {
                                Icon(if (expanded) Icons.Filled.FolderOpen else Icons.Filled.Folder, null,
                                    tint = MaterialTheme.colorScheme.primary)
                            } else {
                                val (icon, tint) = fileIcon(entry.name)
                                Icon(icon, null, tint = tint ?: MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        },
                        headlineContent = { Text(entry.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        trailingContent = if (entry.isDirectory) {
                            {
                                Icon(if (expanded) Icons.Filled.ExpandMore else Icons.AutoMirrored.Filled.KeyboardArrowRight, null)
                            }
                        } else null,
                        modifier = Modifier
                            .padding(start = (row.depth * 20).dp)
                            .clickable {
                                if (entry.isDirectory) {
                                    val toRead = tree.toggle(entry.path)
                                    state.version++
                                    if (toRead != null) scope.launch { read(toRead) }
                                } else {
                                    onOpenFile(entry.path)
                                }
                            },
                    )
                }
            }
        } else {
            val list = state.sessions
            when {
                list == null -> Box(Modifier.fillMaxWidth().padding(32.dp), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator(Modifier.size(32.dp))
                }
                list.isEmpty() -> Text("这个项目还没有 Agent 会话", Modifier.padding(24.dp),
                    color = MaterialTheme.colorScheme.onSurfaceVariant)
                else -> LazyColumn(Modifier.fillMaxHeight()) {
                    items(list, key = { it.provider + ":" + it.id }) { entry ->
                        ListItem(
                            colors = itemColors,
                            leadingContent = { AgentTile(entry.provider, 20.dp) },
                            headlineContent = { Text(entry.title, maxLines = 2, overflow = TextOverflow.Ellipsis) },
                            supportingContent = {
                                Text(
                                    listOfNotNull(
                                        providerName(entry.provider),
                                        entry.model,
                                        DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT)
                                            .format(Date(entry.updatedAtMs.toLong())),
                                    ).joinToString(" · "),
                                )
                            },
                        )
                    }
                }
            }
        }
    }
}
