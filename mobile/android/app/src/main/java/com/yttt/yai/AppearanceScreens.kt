package com.yttt.yai

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import io.github.rosemoe.sora.widget.CodeEditor
import uniffi.yttt_mobile_core.Theme
import uniffi.yttt_mobile_core.theme
import uniffi.yttt_mobile_core.themeChoices
import kotlin.math.roundToInt

/** A theme choice's name; an unknown id shows as following the system. */
fun themeLabel(id: String): String = themeChoices().let { choices -> choices.firstOrNull { it.id == id }?.name ?: choices[0].name }

@Composable
private fun Header(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(start = 16.dp, top = 20.dp, bottom = 4.dp),
    )
}

/** The palette's background, text and six colors side by side. */
@Composable
private fun Swatch(palette: Theme) {
    Row(
        Modifier.clip(RoundedCornerShape(4.dp))
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(4.dp)),
    ) {
        (listOf(palette.background, palette.foreground) + palette.ansi.subList(1, 7)).forEach {
            Box(Modifier.width(8.dp).height(22.dp).background(Color(argb(it))))
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AppearancePage(title: String, onBack: () -> Unit, content: androidx.compose.foundation.lazy.LazyListScope.() -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "返回") } },
                title = { Text(title) },
            )
        },
    ) { padding -> LazyColumn(Modifier.padding(padding).fillMaxWidth(), content = content) }
}

private fun androidx.compose.foundation.lazy.LazyListScope.fontItems(
    font: String,
    size: Float,
    sizes: ClosedFloatingPointRange<Float>,
    onFont: (String) -> Unit,
    onSize: (Float) -> Unit,
) {
    item { Header("字体") }
    AppearanceSettings.FONTS.forEach { (id, name) ->
        item(key = "font-$id") {
            val context = LocalContext.current
            val typeface = remember(id) { AppearanceSettings.typeface(context, id) }
            ListItem(
                headlineContent = { Text(name, fontFamily = FontFamily(typeface)) },
                trailingContent = { RadioButton(selected = font == id, onClick = { onFont(id) }) },
                modifier = Modifier.clickable { onFont(id) },
            )
        }
    }
    item {
        ListItem(
            headlineContent = { Text("字号") },
            supportingContent = {
                Slider(
                    value = size,
                    onValueChange = { onSize(it.roundToInt().toFloat()) },
                    valueRange = sizes,
                    steps = (sizes.endInclusive - sizes.start).toInt() - 1,
                )
            },
            trailingContent = { Text("${size.toInt()}") },
        )
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.themeItems(selected: String, systemDark: Boolean, onTheme: (String) -> Unit) {
    item { Header("主题") }
    themeChoices().forEach { choice ->
        item(key = "theme-${choice.id}") {
            ListItem(
                leadingContent = { Swatch(theme(choice.id, systemDark)) },
                headlineContent = { Text(choice.name) },
                trailingContent = { RadioButton(selected = selected == choice.id, onClick = { onTheme(choice.id) }) },
                modifier = Modifier.clickable { onTheme(choice.id) },
            )
        }
    }
}

/** A few lines of colored output in the terminal's font and colors. */
@Composable
private fun TerminalPreview(look: TerminalAppearance, palette: Theme) {
    val context = LocalContext.current
    val family = remember(look.font) { FontFamily(AppearanceSettings.typeface(context, look.font)) }
    fun c(index: Int) = Color(argb(palette.ansi[index]))
    val text = buildAnnotatedString {
        withStyle(SpanStyle(color = c(4))) { append("~/yttt ") }
        withStyle(SpanStyle(color = c(5))) { append("main ") }
        append("$ cargo test\n")
        withStyle(SpanStyle(color = c(2))) { append("   Compiling ") }
        append("yttt-mobile-core v0.5.7\n")
        append("test result: ")
        withStyle(SpanStyle(color = c(10))) { append("ok") }
        append(". 36 passed; ")
        withStyle(SpanStyle(color = c(1))) { append("0 failed\n") }
        withStyle(SpanStyle(color = c(3))) { append("warning: ") }
        append("unused variable ")
        withStyle(SpanStyle(color = c(6))) { append("`x`\n") }
        append("$ ")
        withStyle(SpanStyle(background = Color(argb(palette.cursor)))) { append(" ") }
    }
    Text(
        text,
        color = Color(argb(palette.foreground)),
        fontFamily = family,
        fontSize = look.size.sp,
        softWrap = false,
        modifier = Modifier.padding(16.dp).fillMaxWidth().clip(RoundedCornerShape(10.dp))
            .background(Color(argb(palette.background))).padding(12.dp),
    )
}

@Composable
fun TerminalAppearanceScreen(store: AppearanceStore, onBack: () -> Unit) {
    val look by store.terminal.collectAsState()
    val systemDark = isSystemInDarkTheme()
    val palette = theme(look.theme, systemDark)
    AppearancePage("终端", onBack) {
        item { TerminalPreview(look, palette) }
        item {
            Text(
                "改字号会改变终端的行列数；接管时电脑上的终端也随之调整。",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
        }
        fontItems(look.font, look.size, AppearanceSettings.TERMINAL_SIZES,
            onFont = { store.setTerminal(look.copy(font = it)) }, onSize = { store.setTerminal(look.copy(size = it)) })
        themeItems(look.theme, systemDark) { store.setTerminal(look.copy(theme = it)) }
    }
}

private const val PREVIEW_CODE = """// 计算斐波那契数
fn fibonacci(n: u32) -> u64 {
    let mut pair = (0, 1);
    for _ in 0..n {
        pair = (pair.1, pair.0 + pair.1);
    }
    pair.0
}

const LIMIT: u32 = 42;"""

@Composable
fun EditorAppearanceScreen(store: AppearanceStore, onBack: () -> Unit) {
    val look by store.editor.collectAsState()
    val systemDark = isSystemInDarkTheme()
    val palette = theme(look.theme, systemDark)
    val context = LocalContext.current
    AppearancePage("编辑器", onBack) {
        item {
            AndroidView(
                factory = { context ->
                    CodeEditor(context).apply {
                        setText(PREVIEW_CODE)
                        isEditable = false
                    }
                },
                update = { editor -> applyLook(editor, context, look, palette, "rust") },
                modifier = Modifier.padding(16.dp).fillMaxWidth().height(220.dp).clip(RoundedCornerShape(10.dp)),
            )
        }
        item {
            Text(
                "按文件扩展名高亮：Rust、Swift、Go、Python、JavaScript / TypeScript、C / C++、Java、JSON、YAML、Markdown、Shell 等。",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
        }
        fontItems(look.font, look.size, AppearanceSettings.EDITOR_SIZES,
            onFont = { store.setEditor(look.copy(font = it)) }, onSize = { store.setEditor(look.copy(size = it)) })
        item { Header("显示") }
        item {
            ListItem(
                headlineContent = { Text("行号") },
                trailingContent = { Switch(look.lineNumbers, onCheckedChange = { store.setEditor(look.copy(lineNumbers = it)) }) },
            )
        }
        item {
            ListItem(
                headlineContent = { Text("自动换行") },
                trailingContent = { Switch(look.wrap, onCheckedChange = { store.setEditor(look.copy(wrap = it)) }) },
            )
        }
        themeItems(look.theme, systemDark) { store.setEditor(look.copy(theme = it)) }
    }
}
