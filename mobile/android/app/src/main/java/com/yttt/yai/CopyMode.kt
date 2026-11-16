package com.yttt.yai

import android.graphics.Typeface
import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.yttt_mobile_core.HostSession
import uniffi.yttt_mobile_core.TextCursor

/** Screens of history read at a time. */
private const val PAGES = 5u

/** How the text view holds its place once new text is laid out. */
private sealed interface TextPlace {
    data object Bottom : TextPlace
    data class Line(val start: Int, val end: Int) : TextPlace
    data class FromBottom(val distance: Int) : TextPlace
}

/**
 * The terminal's text to select and copy, opened by a long press with the
 * pressed line selected. Scrolling up reads further back: the Host's
 * history, or, for a program that scrolls itself, the pages it shows as the
 * phone scrolls it, stitched together. Scrolling the program sends keys, so
 * it needs control: `onTake` takes it.
 */
@Composable
fun CopyModeDialog(
    session: HostSession,
    sessionId: String,
    background: Int,
    foreground: Int,
    typeface: Typeface,
    fontSize: Float,
    controlling: Boolean,
    pressedRow: Int?,
    onTake: suspend () -> Unit,
    onDismiss: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val clipboard = LocalClipboardManager.current
    val context = LocalContext.current
    val inControl by rememberUpdatedState(controlling)
    var lines by remember { mutableStateOf(emptyList<String>()) }
    var older by remember { mutableStateOf<TextCursor?>(null) }
    var alternate by remember { mutableStateOf(false) }
    var atStart by remember { mutableStateOf(false) }
    var note by remember { mutableStateOf<String?>(null) }
    var loading by remember { mutableStateOf(false) }
    var value by remember { mutableStateOf(TextFieldValue("")) }
    var layout by remember { mutableStateOf<TextLayoutResult?>(null) }
    var place by remember { mutableStateOf<TextPlace>(TextPlace.Bottom) }
    // The program's screen last read, to stitch the next page above.
    var screen by remember { mutableStateOf(emptyList<String>()) }
    var firstPage by remember { mutableStateOf(true) }
    // Steps the program was scrolled up, undone on leaving.
    var stepsUp by remember { mutableIntStateOf(0) }
    val scroll = rememberScrollState()
    val focus = remember { FocusRequester() }

    // Text added above: the place and the selection move down with it.
    fun prepend(newLines: List<String>) {
        val text = newLines.joinToString("\n")
        val shift = text.length - value.text.length
        place = TextPlace.FromBottom(scroll.maxValue - scroll.value)
        val selection = value.selection
        lines = newLines
        value = TextFieldValue(
            text,
            if (selection.collapsed) TextRange(text.length)
            else TextRange(selection.start + shift, selection.end + shift),
        )
    }

    suspend fun load() {
        runCatching { session.terminalText(sessionId, PAGES) }
            .onSuccess { text ->
                older = text.older
                alternate = text.alternateScreen
                atStart = !text.alternateScreen && text.older == null
                screen = text.lines
                lines = text.lines
                val joined = text.lines.joinToString("\n")
                val row = pressedRow?.takeIf { it in text.rowLines.indices }
                if (row != null) {
                    val index = text.rowLines[row].toInt()
                    val start = text.lines.take(index).sumOf { it.length + 1 }
                    val end = start + text.lines[index].length
                    place = TextPlace.Line(start, end)
                    value = TextFieldValue(joined, TextRange(start, end))
                } else {
                    place = TextPlace.Bottom
                    value = TextFieldValue(joined)
                }
                note = null
            }
            .onFailure { note = "读取失败：${it.message}" }
    }

    fun loadHistory(cursor: TextCursor) {
        loading = true
        scope.launch {
            runCatching { session.olderText(sessionId, cursor, PAGES) }
                .onSuccess { text ->
                    val merged = text.lines.toMutableList()
                    var rest = lines
                    if (text.lastLineContinues && merged.isNotEmpty() && rest.isNotEmpty()) {
                        merged[merged.lastIndex] = merged.last() + rest.first()
                        rest = rest.drop(1)
                    }
                    prepend(merged + rest)
                    older = text.older
                    atStart = text.older == null
                }
                .onFailure {
                    older = null
                    note = "历史已变化，关闭后重新打开可以重新读取"
                }
            loading = false
        }
    }

    // Scrolls the program up a step and stitches what it shows above.
    fun pageUp() {
        if (!inControl) return
        loading = true
        scope.launch {
            runCatching { session.pageUpText(sessionId, screen) }
                .onSuccess { paged ->
                    if (paged.atTop) {
                        atStart = true
                    } else {
                        stepsUp++
                        // The program's title still starts the text until
                        // the first page is stitched above it.
                        val drop = if (firstPage) paged.header.toInt() else 0
                        firstPage = false
                        prepend(paged.above + lines.drop(drop))
                        screen = paged.screen
                    }
                }
                .onFailure { note = it.message }
            loading = false
        }
    }

    fun loadOlder() {
        if (loading || atStart) return
        if (alternate) pageUp() else older?.let(::loadHistory)
    }

    LaunchedEffect(Unit) { load() }
    // Once the new text is laid out, hold the place.
    LaunchedEffect(lines) {
        withFrameNanos {}
        when (val at = place) {
            TextPlace.Bottom -> scroll.scrollTo(scroll.maxValue)
            is TextPlace.FromBottom -> scroll.scrollTo((scroll.maxValue - at.distance).coerceAtLeast(0))
            is TextPlace.Line -> {
                val top = layout?.let { it.getLineTop(it.getLineForOffset(at.start)) }?.toInt() ?: 0
                scroll.scrollTo((top - scroll.viewportSize / 2).coerceIn(0, scroll.maxValue))
                runCatching { focus.requestFocus() }
            }
        }
    }
    // Near the top: read further back.
    LaunchedEffect(scroll) {
        snapshotFlow { scroll.value }.collect { offset ->
            if (offset < scroll.viewportSize) loadOlder()
        }
    }
    // Puts the program back where it was once copying ends; one more step
    // for a last one that moved less, as the program stops at its end.
    DisposableEffect(Unit) {
        onDispose {
            if (stepsUp > 0 && inControl) runCatching { session.scrollProgram(sessionId, stepsUp + 1) }
        }
    }

    val banner = when {
        note != null -> note
        atStart -> if (alternate) "已到程序内容的开头" else "已到历史开头"
        alternate && inControl -> "往上滑会让程序往回翻页，拼接更早的内容"
        alternate -> "接管后往上滑，可以加载程序更早的内容"
        else -> null
    }

    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surface) {
            Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing)) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text("复制", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(8.dp).weight(1f))
                    TextButton(onClick = onDismiss) { Text("完成") }
                }
                if (banner != null) {
                    Row(
                        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 2.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(banner, style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(1f))
                        if (alternate && !inControl && !atStart) {
                            TextButton(onClick = {
                                scope.launch {
                                    onTake()
                                    // At the phone's size the program redraws:
                                    // the text starts over from its screen.
                                    delay(600)
                                    firstPage = true
                                    stepsUp = 0
                                    load()
                                }
                            }) { Text("接管") }
                        } else if (alternate && inControl && !atStart && scroll.maxValue == 0) {
                            // Nothing to scroll yet: page by hand.
                            TextButton(onClick = ::loadOlder) { Text("加载更早") }
                        }
                    }
                }
                Box(Modifier.weight(1f).fillMaxWidth().background(Color(background))) {
                    BasicTextField(
                        value = value,
                        onValueChange = { value = it.copy(text = value.text) },
                        readOnly = true,
                        textStyle = TextStyle(
                            color = Color(foreground),
                            fontFamily = FontFamily(typeface),
                            fontSize = fontSize.sp,
                        ),
                        cursorBrush = SolidColor(Color.Transparent),
                        onTextLayout = { layout = it },
                        modifier = Modifier.fillMaxSize().verticalScroll(scroll).padding(8.dp).focusRequester(focus),
                    )
                }
                Row(
                    Modifier.fillMaxWidth().padding(8.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    OutlinedButton(onClick = {
                        value = value.copy(selection = TextRange(0, value.text.length))
                        runCatching { focus.requestFocus() }
                    }) { Text("全选") }
                    Box(Modifier.weight(1f))
                    Button(onClick = {
                        val selection = value.selection
                        val text = if (selection.collapsed) value.text
                        else value.text.substring(selection.min, selection.max)
                        clipboard.setText(AnnotatedString(text))
                        Toast.makeText(context, "已复制", Toast.LENGTH_SHORT).show()
                    }) { Text("复制") }
                }
            }
        }
    }
}
