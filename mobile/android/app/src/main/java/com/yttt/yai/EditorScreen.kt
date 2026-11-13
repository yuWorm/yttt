package com.yttt.yai

import android.graphics.Typeface
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.viewinterop.AndroidView
import io.github.rosemoe.sora.event.ContentChangeEvent
import io.github.rosemoe.sora.widget.CodeEditor
import kotlinx.coroutines.launch
import uniffi.yttt_mobile_core.HostSession
import uniffi.yttt_mobile_core.SaveOutcome

/**
 * A project file, full screen, in sora-editor. Saving checks the file is as
 * it was when opened; if the Host's copy changed, the user overwrites it or
 * reloads it. Closing with unsaved changes asks first.
 */
@Composable
fun EditorScreen(session: HostSession, projectId: String, path: String, onClose: () -> Unit) {
    var editor by remember { mutableStateOf<CodeEditor?>(null) }
    var loaded by remember { mutableStateOf<String?>(null) }
    var dirty by remember { mutableStateOf(false) }
    var status by remember { mutableStateOf("") }
    var conflict by remember { mutableStateOf(false) }
    var confirmClose by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    suspend fun load() {
        runCatching { session.openFile(projectId, path) }
            .onSuccess {
                loaded = it.text
                editor?.setText(it.text)
                dirty = false
                status = ""
            }
            .onFailure { status = "打不开：${it.message}" }
    }

    suspend fun save(force: Boolean) {
        val text = editor?.text?.toString() ?: return
        runCatching { session.saveFile(projectId, path, text, force) }
            .onSuccess {
                when (it) {
                    SaveOutcome.SAVED -> { dirty = false; status = "已保存" }
                    SaveOutcome.CONFLICT -> conflict = true
                }
            }
            .onFailure { status = "保存失败：${it.message}" }
    }

    LaunchedEffect(projectId, path) { load() }
    BackHandler { if (dirty) confirmClose = true else onClose() }

    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = { if (dirty) confirmClose = true else onClose() }) { Text("关闭") }
            Column(Modifier.weight(1f)) {
                Text(path.substringAfterLast('/') + if (dirty) " ●" else "", style = MaterialTheme.typography.titleMedium,
                    maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (status.isNotEmpty()) Text(status, style = MaterialTheme.typography.bodySmall)
            }
            TextButton(onClick = { scope.launch { save(force = false) } }, enabled = dirty) { Text("保存") }
        }
        Box(Modifier.fillMaxSize()) {
            AndroidView(
                factory = { context ->
                    CodeEditor(context).apply {
                        typefaceText = Typeface.MONOSPACE
                        loaded?.let { setText(it) }
                        subscribeEvent(ContentChangeEvent::class.java) { event, _ ->
                            // setText reports a whole-text change too.
                            if (event.action != ContentChangeEvent.ACTION_SET_NEW_TEXT) dirty = true
                        }
                        editor = this
                    }
                },
                modifier = Modifier.fillMaxSize(),
            )
            if (loaded == null && status.isEmpty()) Text("正在读取…", Modifier.align(Alignment.Center))
        }
    }

    if (conflict) {
        AlertDialog(
            onDismissRequest = { conflict = false },
            title = { Text("文件在电脑上改过了") },
            text = { Text("覆盖会用手机上的内容替换电脑上的版本；重新加载会丢掉手机上的修改。") },
            confirmButton = {
                TextButton(onClick = { conflict = false; scope.launch { save(force = true) } }) { Text("覆盖") }
            },
            dismissButton = {
                TextButton(onClick = { conflict = false; scope.launch { load() } }) { Text("重新加载") }
            },
        )
    }
    if (confirmClose) {
        AlertDialog(
            onDismissRequest = { confirmClose = false },
            title = { Text("放弃修改？") },
            text = { Text("这个文件还有没保存的修改。") },
            confirmButton = { TextButton(onClick = { confirmClose = false; onClose() }) { Text("放弃") } },
            dismissButton = { TextButton(onClick = { confirmClose = false }) { Text("继续编辑") } },
        )
    }
}
