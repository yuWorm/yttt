package com.yttt.yai

import android.content.Context
import android.content.SharedPreferences
import android.graphics.Typeface
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONArray
import org.json.JSONObject
import uniffi.yttt_mobile_core.Theme

/** The appearance settings' ranges, defaults and fonts. */
object AppearanceSettings {
    const val DEFAULT_FONT = "system"
    const val DEFAULT_THEME = "system"
    const val DEFAULT_TERMINAL_SIZE = 11f
    const val DEFAULT_EDITOR_SIZE = 14f
    val TERMINAL_SIZES = 8f..20f
    val EDITOR_SIZES = 10f..24f

    fun terminalSize(size: Float) = size.coerceIn(TERMINAL_SIZES)
    fun editorSize(size: Float) = size.coerceIn(EDITOR_SIZES)

    /** JetBrains Mono comes with the app (OFL, `mobile/fonts`). */
    val FONTS = listOf("system" to "系统等宽", "jetbrains" to "JetBrains Mono")

    fun fontName(id: String) = FONTS.firstOrNull { it.first == id }?.second ?: FONTS[0].second

    /** The asset holding a font, or null for the system's monospace. */
    fun fontAsset(id: String, bold: Boolean): String? = when (id) {
        "jetbrains" -> if (bold) "JetBrainsMono-Bold.ttf" else "JetBrainsMono-Regular.ttf"
        else -> null
    }

    fun typeface(context: Context, id: String, bold: Boolean = false): Typeface =
        fontAsset(id, bold)?.let { asset -> runCatching { Typeface.createFromAsset(context.assets, asset) }.getOrNull() }
            ?: Typeface.create(Typeface.MONOSPACE, if (bold) Typeface.BOLD else Typeface.NORMAL)
}

data class TerminalAppearance(
    val font: String = AppearanceSettings.DEFAULT_FONT,
    val size: Float = AppearanceSettings.DEFAULT_TERMINAL_SIZE,
    val theme: String = AppearanceSettings.DEFAULT_THEME,
)

data class EditorAppearance(
    val font: String = AppearanceSettings.DEFAULT_FONT,
    val size: Float = AppearanceSettings.DEFAULT_EDITOR_SIZE,
    val theme: String = AppearanceSettings.DEFAULT_THEME,
    val wrap: Boolean = false,
    val lineNumbers: Boolean = true,
)

/** The settings in SharedPreferences, as flows the screens follow. */
class AppearanceStore(private val prefs: SharedPreferences) {
    private val terminalFlow = MutableStateFlow(readTerminal())
    private val editorFlow = MutableStateFlow(readEditor())
    val terminal: StateFlow<TerminalAppearance> = terminalFlow
    val editor: StateFlow<EditorAppearance> = editorFlow

    private fun readTerminal() = TerminalAppearance(
        font = prefs.getString("terminal.font", null) ?: AppearanceSettings.DEFAULT_FONT,
        size = AppearanceSettings.terminalSize(prefs.getFloat("terminal.fontSize", AppearanceSettings.DEFAULT_TERMINAL_SIZE)),
        theme = prefs.getString("terminal.theme", null) ?: AppearanceSettings.DEFAULT_THEME,
    )

    private fun readEditor() = EditorAppearance(
        font = prefs.getString("editor.font", null) ?: AppearanceSettings.DEFAULT_FONT,
        size = AppearanceSettings.editorSize(prefs.getFloat("editor.fontSize", AppearanceSettings.DEFAULT_EDITOR_SIZE)),
        theme = prefs.getString("editor.theme", null) ?: AppearanceSettings.DEFAULT_THEME,
        wrap = prefs.getBoolean("editor.wrap", false),
        lineNumbers = prefs.getBoolean("editor.lineNumbers", true),
    )

    fun setTerminal(value: TerminalAppearance) {
        prefs.edit().putString("terminal.font", value.font)
            .putFloat("terminal.fontSize", AppearanceSettings.terminalSize(value.size))
            .putString("terminal.theme", value.theme).apply()
        terminalFlow.value = readTerminal()
    }

    fun setEditor(value: EditorAppearance) {
        prefs.edit().putString("editor.font", value.font)
            .putFloat("editor.fontSize", AppearanceSettings.editorSize(value.size))
            .putString("editor.theme", value.theme)
            .putBoolean("editor.wrap", value.wrap)
            .putBoolean("editor.lineNumbers", value.lineNumbers).apply()
        editorFlow.value = readEditor()
    }
}

/** `0xRRGGBB` as an opaque ARGB int. */
fun argb(rgb: UInt): Int = (0xFF000000u or rgb).toInt()

fun hex(rgb: UInt): String = "#%06X".format(rgb.toInt() and 0xFFFFFF)

/**
 * A core palette as a TextMate theme for sora-editor: its editor colors, and
 * token rules for the scopes VS Code's grammars use. Later rules win.
 */
fun textMateTheme(theme: Theme): String {
    val syntax = theme.syntax
    fun rule(scopes: List<String>, color: UInt, style: String? = null) = JSONObject()
        .put("scope", JSONArray(scopes))
        .put("settings", JSONObject().put("foreground", hex(color)).apply { style?.let { put("fontStyle", it) } })
    val rules = JSONArray()
        .put(JSONObject().put("settings", JSONObject().put("foreground", hex(theme.foreground)).put("background", hex(theme.background))))
        .put(rule(listOf("constant", "variable.other.constant", "support.constant", "entity.other.attribute-name",
            "variable.other.property", "support.type.property-name"), syntax.constant))
        .put(rule(listOf("entity.name.type", "entity.name.class", "support.type", "support.class", "entity.name.tag",
            "entity.name.namespace", "storage.type.primitive"), syntax.typeName))
        .put(rule(listOf("entity.name.function", "support.function", "meta.function-call", "entity.name.method"), syntax.function))
        .put(rule(listOf("constant.numeric", "constant.language.boolean", "constant.language"), syntax.number))
        .put(rule(listOf("string", "punctuation.definition.string", "markup.inline.raw", "markup.fenced_code"), syntax.string))
        .put(rule(listOf("keyword", "keyword.control", "keyword.other", "storage", "storage.type", "storage.modifier"), syntax.keyword))
        .put(rule(listOf("markup.heading", "entity.name.section"), syntax.keyword, "bold"))
        .put(rule(listOf("markup.underline.link", "string.other.link"), syntax.function))
        .put(rule(listOf("comment", "punctuation.definition.comment"), syntax.comment, "italic"))
    val colors = JSONObject()
        .put("editor.background", hex(theme.background))
        .put("editor.foreground", hex(theme.foreground))
        .put("editor.lineHighlightBackground", hex(theme.selection) + "60")
        .put("editor.selectionBackground", hex(theme.selection))
        .put("editorCursor.foreground", hex(theme.cursor))
        .put("editorLineNumber.foreground", hex(syntax.comment))
        .put("editorLineNumber.activeForeground", hex(theme.foreground))
        .put("editorIndentGuide.background", hex(syntax.comment) + "40")
        .put("editorWhitespace.foreground", hex(syntax.comment) + "60")
    return JSONObject()
        .put("name", theme.id)
        .put("type", if (theme.dark) "dark" else "light")
        .put("colors", colors)
        .put("tokenColors", rules)
        .toString()
}
