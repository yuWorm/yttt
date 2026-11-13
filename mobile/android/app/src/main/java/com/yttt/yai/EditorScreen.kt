package com.yttt.yai

import uniffi.yttt_mobile_core.theme

import uniffi.yttt_mobile_core.editorLanguage

import uniffi.yttt_mobile_core.Theme

import androidx.compose.runtime.collectAsState

import androidx.compose.ui.platform.LocalContext

import androidx.compose.foundation.isSystemInDarkTheme

import android.content.Context

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.outlined.Save
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.viewinterop.AndroidView
import io.github.rosemoe.sora.event.ContentChangeEvent
import io.github.rosemoe.sora.widget.CodeEditor
import kotlinx.coroutines.launch
import uniffi.yttt_mobile_core.HostSession
import uniffi.yttt_mobile_core.SaveOutcome

/**
 * A project file, full screen, in sora-editor. Saving checks the file is as
 * it was when opened; if the Host's copy changed, the user overwrites it or
 * reloads it. Closing or going back with unsaved changes asks first.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun EditorScreen(
    session: HostSession,
    appearance: AppearanceStore,
    projectId: String,
    projectName: String,
    path: String,
    onClose: () -> Unit,
) {
    val look by appearance.editor.collectAsState()
    val systemDark = isSystemInDarkTheme()
    val palette = remember(look.theme, systemDark) { theme(look.theme, systemDark) }
    val context = LocalContext.current
    val language = remember(path) { editorLanguage(path) }
    var editor by remember { mutableStateOf<CodeEditor?>(null) }
    var loaded by remember { mutableStateOf<String?>(null) }
    var dirty by remember { mutableStateOf(false) }
    var failed by remember { mutableStateOf<String?>(null) }
    var conflict by remember { mutableStateOf(false) }
    var confirmClose by remember { mutableStateOf(false) }
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()

    suspend fun load() {
        runCatching { session.openFile(projectId, path) }
            .onSuccess {
                loaded = it.text
                editor?.setText(it.text)
                dirty = false
                failed = null
            }
            .onFailure { failed = "打不开：${it.message}" }
    }

    suspend fun save(force: Boolean) {
        val text = editor?.text?.toString() ?: return
        runCatching { session.saveFile(projectId, path, text, force) }
            .onSuccess {
                when (it) {
                    SaveOutcome.SAVED -> { dirty = false; snackbar.showSnackbar("已保存") }
                    SaveOutcome.CONFLICT -> conflict = true
                }
            }
            .onFailure { snackbar.showSnackbar("保存失败：${it.message}") }
    }

    fun close() = if (dirty) confirmClose = true else onClose()

    LaunchedEffect(projectId, path) { load() }
    BackHandler(enabled = dirty) { confirmClose = true }

    Scaffold(
        // Above the keyboard, where saving usually happens.
        snackbarHost = { SnackbarHost(snackbar, Modifier.imePadding()) },
        topBar = {
            TopAppBar(
                navigationIcon = { IconButton(onClick = ::close) { Icon(Icons.Filled.Close, "关闭") } },
                title = {
                    Column {
                        Text(path.substringAfterLast('/') + if (dirty) " ●" else "", maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text("$projectName / $path", style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                },
                actions = {
                    IconButton(onClick = { scope.launch { save(force = false) } }, enabled = dirty) {
                        Icon(Icons.Outlined.Save, "保存")
                    }
                },
            )
        },
    ) { padding ->
        Box(Modifier.padding(padding).consumeWindowInsets(padding).fillMaxSize().imePadding()) {
            AndroidView(
                factory = { context ->
                    CodeEditor(context).apply {
                        setEditorLanguage(TextMate.language(context, language))
                        loaded?.let { setText(it) }
                        subscribeEvent(ContentChangeEvent::class.java) { event, _ ->
                            // setText reports a whole-text change too.
                            if (event.action != ContentChangeEvent.ACTION_SET_NEW_TEXT) dirty = true
                        }
                        editor = this
                    }
                },
                update = { editor -> applyLook(editor, context, look, palette) },
                modifier = Modifier.fillMaxSize(),
            )
            when {
                failed != null -> Text(failed!!, Modifier.align(Alignment.Center), color = MaterialTheme.colorScheme.error)
                loaded == null -> CircularProgressIndicator(Modifier.align(Alignment.Center))
            }
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

/** The editor's font, size, wrapping, line numbers and colors, applied only when they change. */
fun applyLook(editor: CodeEditor, context: Context, look: EditorAppearance, palette: Theme) {
    val key = look to palette.id
    if (editor.tag == key) return
    editor.tag = key
    editor.typefaceText = AppearanceSettings.typeface(context, look.font)
    editor.typefaceLineNumber = AppearanceSettings.typeface(context, look.font)
    editor.setTextSize(look.size)
    editor.isWordwrap = look.wrap
    editor.isLineNumberEnabled = look.lineNumbers
    editor.colorScheme = TextMate.colorScheme(context, palette)
}
