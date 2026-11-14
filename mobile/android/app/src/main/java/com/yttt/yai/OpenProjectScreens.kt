package com.yttt.yai

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.outlined.DriveFolderUpload
import androidx.compose.material.icons.outlined.History
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import uniffi.yttt_mobile_core.HostDirectory
import uniffi.yttt_mobile_core.HostSession
import uniffi.yttt_mobile_core.RecentProject

/**
 * Chooses a folder on the Host, starting at its home, and opens it as a
 * project. [onOpen] answers null once open, else what went wrong.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HostFolderScreen(session: HostSession, onBack: () -> Unit, onOpen: suspend (String) -> String?) {
    var directory by remember { mutableStateOf<HostDirectory?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var opening by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    fun load(path: String) {
        scope.launch {
            runCatching { session.browseHost(path) }
                .onSuccess { directory = it; error = null }
                .onFailure { error = it.message }
        }
    }
    LaunchedEffect(session) {
        runCatching { session.hostHome() }.onSuccess { load(it) }.onFailure { error = it.message }
    }
    // Back goes up a folder before it leaves.
    BackHandler(enabled = directory?.parent != null) { directory?.parent?.let { load(it) } }

    Scaffold(
        contentWindowInsets = WindowInsets(0),
        topBar = {
            TopAppBar(
                title = { Text("从目录打开") },
                navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "返回") } },
            )
        },
        bottomBar = {
            directory?.let { current ->
                Surface(tonalElevation = 3.dp) {
                    Button(
                        onClick = {
                            scope.launch {
                                opening = true
                                error = onOpen(current.path)
                                opening = false
                            }
                        },
                        enabled = !opening,
                        modifier = Modifier.fillMaxWidth().navigationBarsPadding().padding(16.dp),
                    ) {
                        if (opening) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                        else Text("打开这个目录")
                    }
                }
            }
        },
    ) { padding ->
        Column(Modifier.padding(padding).fillMaxSize()) {
            error?.let { Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(16.dp)) }
            val current = directory
            if (current == null) {
                if (error == null) Loading()
                return@Column
            }
            Text(
                current.path,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.StartEllipsis,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            )
            LazyColumn(Modifier.fillMaxWidth()) {
                current.parent?.let { parent ->
                    item(key = "up") {
                        ListItem(
                            leadingContent = { Icon(Icons.Outlined.DriveFolderUpload, null) },
                            headlineContent = { Text("上一级") },
                            modifier = Modifier.clickable { load(parent) },
                        )
                        HorizontalDivider()
                    }
                }
                if (current.folders.isEmpty()) {
                    item(key = "empty") {
                        Text("这里没有子目录", color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(16.dp))
                    }
                }
                items(current.folders, key = { it.path }) { folder ->
                    ListItem(
                        leadingContent = { Icon(Icons.Filled.Folder, null, tint = runningBlue) },
                        headlineContent = { Text(folder.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        trailingContent = { Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, null) },
                        modifier = Modifier.clickable { load(folder.path) },
                    )
                }
            }
        }
    }
}

/** The projects the Host opened before; [onOpen] answers null once open, else what went wrong. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RecentProjectsScreen(session: HostSession, onBack: () -> Unit, onOpen: suspend (String) -> String?) {
    var projects by remember { mutableStateOf<List<RecentProject>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var opening by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(session) {
        runCatching { session.recentProjects() }.onSuccess { projects = it }.onFailure { error = it.message }
    }

    Scaffold(
        contentWindowInsets = WindowInsets(0),
        topBar = {
            TopAppBar(
                title = { Text("从历史打开") },
                navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "返回") } },
            )
        },
    ) { padding ->
        Column(Modifier.padding(padding).fillMaxSize()) {
            error?.let { Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(16.dp)) }
            val list = projects
            when {
                list == null -> if (error == null) Loading()
                list.isEmpty() -> Column(
                    Modifier.fillMaxSize().padding(32.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically),
                ) {
                    Icon(Icons.Outlined.History, null, Modifier.size(48.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                    Text("没有打开过的项目", style = MaterialTheme.typography.titleMedium)
                    Text("在电脑上打开过的项目会出现在这里。", color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                else -> LazyColumn(Modifier.fillMaxWidth()) {
                    items(list, key = { it.path }) { project ->
                        ListItem(
                            leadingContent = { Icon(Icons.Outlined.History, null) },
                            headlineContent = { Text(project.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                            supportingContent = {
                                Text(project.path, maxLines = 1, overflow = TextOverflow.StartEllipsis)
                            },
                            trailingContent = {
                                when {
                                    opening == project.path ->
                                        CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                                    project.openProjectId != null ->
                                        Text("已打开", style = MaterialTheme.typography.labelSmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                            },
                            modifier = Modifier.clickable(enabled = opening == null) {
                                scope.launch {
                                    opening = project.path
                                    error = onOpen(project.path)
                                    opening = null
                                }
                            },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun Loading() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
}
