package com.yttt.yai

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.yttt_mobile_core.SyntaxColors
import uniffi.yttt_mobile_core.Theme

class AppearanceTest {
    private val palette = Theme(
        id = "nord", name = "Nord", dark = true,
        background = 0x2E3440u, foreground = 0xD8DEE9u, cursor = 0xD8DEE9u, selection = 0x434C5Eu,
        ansi = List(16) { (0x101010 * it).toUInt() },
        syntax = SyntaxColors(
            comment = 0x7B88A1u, keyword = 0x81A1C1u, string = 0xA3BE8Cu, number = 0xB48EADu,
            function = 0x88C0D0u, typeName = 0x8FBCBBu, constant = 0xEBCB8Bu,
        ),
    )

    @Test
    fun sizesStayInRangeAndDefaultsAreWhatTheAppShowedBefore() {
        assertEquals(8f, AppearanceSettings.terminalSize(3f))
        assertEquals(20f, AppearanceSettings.terminalSize(40f))
        assertEquals(13f, AppearanceSettings.terminalSize(13f))
        assertEquals(10f, AppearanceSettings.editorSize(1f))
        assertEquals(24f, AppearanceSettings.editorSize(30f))
        assertEquals("system", AppearanceSettings.DEFAULT_THEME)
        assertEquals("system", AppearanceSettings.DEFAULT_FONT)
    }

    @Test
    fun unknownFontsFallBackToTheSystemMonospace() {
        assertEquals("系统等宽", AppearanceSettings.fontName("from-a-later-version"))
        assertEquals("JetBrains Mono", AppearanceSettings.fontName("jetbrains"))
        assertEquals(null, AppearanceSettings.fontAsset("system", bold = false))
        assertEquals("JetBrainsMono-Bold.ttf", AppearanceSettings.fontAsset("jetbrains", bold = true))
    }

    @Test
    fun colorsGainAnOpaqueAlpha() {
        assertEquals(0xFF2E3440.toInt(), argb(0x2E3440u))
        assertEquals("#2E3440", hex(0x2E3440u))
    }

    @Test
    fun aPaletteBecomesATextMateTheme() {
        val json = JSONObject(textMateTheme(palette))
        assertEquals("nord", json.getString("name"))
        assertEquals("dark", json.getString("type"))
        val colors = json.getJSONObject("colors")
        assertEquals("#2E3440", colors.getString("editor.background"))
        assertEquals("#D8DEE9", colors.getString("editor.foreground"))
        assertEquals("#7B88A1", colors.getString("editorLineNumber.foreground"))
        val rules = json.getJSONArray("tokenColors")
        fun colorOf(scope: String): String? {
            var found: String? = null
            for (i in 0 until rules.length()) {
                val rule = rules.getJSONObject(i)
                val scopes = rule.optJSONArray("scope") ?: continue
                if ((0 until scopes.length()).any { scopes.getString(it) == scope }) {
                    found = rule.getJSONObject("settings").optString("foreground")
                }
            }
            return found
        }
        assertEquals("#7B88A1", colorOf("comment"))
        assertEquals("#81A1C1", colorOf("keyword"))
        assertEquals("#A3BE8C", colorOf("string"))
        assertEquals("#B48EAD", colorOf("constant.numeric"))
        assertEquals("#88C0D0", colorOf("entity.name.function"))
        assertEquals("#8FBCBB", colorOf("entity.name.type"))
        assertEquals("#EBCB8B", colorOf("constant"))
        // The default text color comes first, for tokens no rule names.
        assertTrue(!rules.getJSONObject(0).has("scope"))
        assertEquals("#D8DEE9", rules.getJSONObject(0).getJSONObject("settings").getString("foreground"))
    }
}
