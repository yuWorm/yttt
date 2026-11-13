package com.yttt.yai

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Bolt
import androidx.compose.material.icons.filled.PanTool
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material3.Badge
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import uniffi.yttt_mobile_core.Overview

/** The projects, each with its running Agents' logos, waits and runs. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ProjectsScreen(overview: Overview, onProject: (String) -> Unit) {
    val badges = overview.badges()
    Scaffold(contentWindowInsets = WindowInsets(0), topBar = { TopAppBar(title = { Text("项目") }) }) { padding ->
        if (overview.projects.isEmpty()) {
            Column(
                Modifier.padding(padding).fillMaxSize().padding(32.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically),
            ) {
                Icon(Icons.Outlined.Folder, null, Modifier.size(48.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                Text("电脑上还没有打开的项目", style = MaterialTheme.typography.titleMedium)
            }
            return@Scaffold
        }
        LazyColumn(Modifier.padding(padding).fillMaxWidth()) {
            items(overview.projects, key = { it.projectId }) { project ->
                val mark = overview.projectMark(project.projectId)
                val badge = badges[project.projectId]
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.Folder, null) },
                    headlineContent = { Text(project.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    supportingContent = { Text("${project.tabs.size} 个 tab") },
                    trailingContent = {
                        Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                            for (provider in mark.providers.take(3)) AgentLogo(provider, 18.dp)
                            if (mark.waiting > 0) {
                                Badge(containerColor = MaterialTheme.colorScheme.error) {
                                    Icon(Icons.Filled.PanTool, null, Modifier.size(10.dp))
                                    Text(" ${mark.waiting}")
                                }
                            }
                            badge?.running?.takeIf { it > 0 }?.let {
                                Badge(
                                    containerColor = MaterialTheme.colorScheme.secondaryContainer,
                                    contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
                                ) {
                                    Icon(Icons.Filled.Bolt, null, Modifier.size(10.dp))
                                    Text(" $it")
                                }
                            }
                        }
                    },
                    modifier = Modifier.clickable { onProject(project.projectId) },
                )
            }
        }
    }
}
