package com.yttt.yai

import org.eclipse.tm4e.core.registry.IGrammarSource
import org.eclipse.tm4e.core.registry.IRegistryOptions
import org.eclipse.tm4e.core.registry.IThemeSource
import org.eclipse.tm4e.core.registry.Registry
import org.junit.Assert.assertEquals
import org.junit.Test
import java.io.File
import uniffi.yttt_mobile_core.SyntaxColors
import uniffi.yttt_mobile_core.Theme

/** The bundled grammars and a generated theme color code as the palette says. */
class TextMateScopesTest {
    private val palette = Theme(
        "light", "亮色", false, 0xFFFFFFu, 0x333333u, 0u, 0xADD6FFu, List(16) { 0u },
        SyntaxColors(0x008000u, 0xAF00DBu, 0xA31515u, 0x098658u, 0x795E26u, 0x267F99u, 0x0070C1u),
    )

    /** Each token's text and the color the theme gives it. */
    private fun colors(grammarFile: String, line: String): List<Pair<String, String?>> {
        val registry = Registry(object : IRegistryOptions {})
        registry.setTheme(IThemeSource.fromString(IThemeSource.ContentType.JSON, textMateTheme(palette)))
        val grammar = registry.addGrammar(IGrammarSource.fromFile(File("src/main/assets/textmate/$grammarFile")))
        val tokens = grammar.tokenizeLine2(line).tokens
        return (tokens.indices step 2).map { i ->
            val end = if (i + 2 < tokens.size) tokens[i + 2] else line.length
            line.substring(tokens[i], end) to registry.colorMap.getOrNull((tokens[i + 1] ushr 15) and 0x1FF)
        }
    }

    /** Tokens with the same color merge, as `// done`: the first holding `text`. */
    private fun colorOf(tokens: List<Pair<String, String?>>, text: String) =
        (tokens.firstOrNull { it.first.trim() == text } ?: tokens.first { text in it.first }).second

    @Test
    fun rustIsColoredByRole() {
        val tokens = colors("rust.tmLanguage.json", "fn fibonacci(n: u32) -> u64 { let s = \"x\"; 42 } // done")
        assertEquals("#AF00DB", colorOf(tokens, "fn"))
        assertEquals("#795E26", colorOf(tokens, "fibonacci"))
        assertEquals("#267F99", colorOf(tokens, "u32"))
        assertEquals("#A31515", colorOf(tokens, "x"))
        assertEquals("#098658", colorOf(tokens, "42"))
        assertEquals("#008000", colorOf(tokens, "done"))
    }

    @Test
    fun pythonCommentsAndKeywordsToo() {
        val tokens = colors("python.tmLanguage.json", "def run(): return None  # later")
        assertEquals("#AF00DB", colorOf(tokens, "def"))
        assertEquals("#795E26", colorOf(tokens, "run"))
        assertEquals("#008000", colorOf(tokens, "later"))
    }
}
