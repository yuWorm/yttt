package com.yttt.yai

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import uniffi.yttt_mobile_core.HostSession
import uniffi.yttt_mobile_core.SessionEntry
import java.text.DateFormat
import java.util.Date

/**
 * One project's right drawer: kept above the drawer, which leaves the
 * screen while a file is open, so the tree is as the user left it.
 */
class FilesState {
    val tree = FileTree()
    var version by mutableIntStateOf(0)
    var section by mutableIntStateOf(0)
    var sessions by mutableStateOf<List<SessionEntry>?>(null)
    var error by mutableStateOf<String?>(null)
    var rootRead = false
}

/** The right drawer: the project's files, a directory at a time, and its Agent sessions. */
@Composable
fun FilesDrawer(session: HostSession, projectId: String, state: FilesState, onOpenFile: (String) -> Unit) {
    val tree = state.tree
    val version = state.version
    val section = state.section
    val error = state.error
    val sessions = state.sessions
    val scope = rememberCoroutineScope()

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
    LaunchedEffect(projectId, section) {
        if (section == 1 && state.sessions == null) {
            state.sessions = runCatching { session.agentSessions(projectId) }
                .onFailure { state.error = it.message }
                .getOrDefault(emptyList())
        }
    }

    ModalDrawerSheet {
        Column(Modifier.fillMaxSize()) {
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(16.dp)) {
                listOf("文件", "会话").forEachIndexed { index, label ->
                    SegmentedButton(
                        selected = section == index,
                        onClick = { state.section = index },
                        shape = SegmentedButtonDefaults.itemShape(index, 2),
                    ) { Text(label) }
                }
            }
            error?.let { Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(16.dp)) }
            if (section == 0) {
                // Read `version` so the list follows the tree's changes.
                val rows = version.let { tree.rows() }
                LazyColumn {
                    items(rows, key = { it.entry.path }) { row ->
                        ListItem(
                            headlineContent = {
                                Text(
                                    (if (row.entry.isDirectory) (if (tree.isExpanded(row.entry.path)) "▾ " else "▸ ") else "   ") +
                                        row.entry.name,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            },
                            modifier = Modifier
                                .padding(start = (row.depth * 16).dp)
                                .clickable {
                                    if (row.entry.isDirectory) {
                                        val toRead = tree.toggle(row.entry.path)
                                        state.version++
                                        if (toRead != null) scope.launch { read(toRead) }
                                    } else {
                                        onOpenFile(row.entry.path)
                                    }
                                },
                        )
                    }
                }
            } else {
                val list = sessions
                when {
                    list == null -> Text("正在读取…", Modifier.padding(16.dp))
                    list.isEmpty() -> Text("这个项目还没有 Agent 会话", Modifier.padding(16.dp))
                    else -> LazyColumn {
                        items(list, key = { it.provider + ":" + it.id }) { entry ->
                            ListItem(
                                headlineContent = { Text(entry.title, maxLines = 2, overflow = TextOverflow.Ellipsis) },
                                supportingContent = {
                                    Text(
                                        listOfNotNull(
                                            entry.provider,
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
}
