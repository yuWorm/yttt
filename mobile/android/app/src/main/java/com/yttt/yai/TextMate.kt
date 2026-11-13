package com.yttt.yai

import android.content.Context
import io.github.rosemoe.sora.lang.EmptyLanguage
import io.github.rosemoe.sora.lang.Language
import io.github.rosemoe.sora.langs.textmate.TextMateColorScheme
import io.github.rosemoe.sora.langs.textmate.TextMateLanguage
import io.github.rosemoe.sora.langs.textmate.registry.FileProviderRegistry
import io.github.rosemoe.sora.langs.textmate.registry.GrammarRegistry
import io.github.rosemoe.sora.langs.textmate.registry.ThemeRegistry
import io.github.rosemoe.sora.langs.textmate.registry.model.ThemeModel
import io.github.rosemoe.sora.langs.textmate.registry.provider.AssetsFileResolver
import org.eclipse.tm4e.core.registry.IThemeSource
import org.json.JSONObject
import uniffi.yttt_mobile_core.Theme

/**
 * The editor's highlighting: VS Code's TextMate grammars from
 * assets/textmate, colored by the core's palettes turned into TextMate
 * themes. The registries are process-wide, set up once.
 */
object TextMate {
    private var scopes: Map<String, String>? = null

    private fun ensure(context: Context): Map<String, String> = scopes ?: synchronized(this) {
        scopes ?: run {
            val assets = context.applicationContext.assets
            FileProviderRegistry.getInstance().addFileProvider(AssetsFileResolver(assets))
            GrammarRegistry.getInstance().loadGrammars("textmate/languages.json")
            val languages = JSONObject(assets.open("textmate/languages.json").bufferedReader().readText()).getJSONArray("languages")
            (0 until languages.length()).associate {
                val language = languages.getJSONObject(it)
                language.getString("name") to language.getString("scopeName")
            }.also { scopes = it }
        }
    }

    /** A color scheme for `theme`; the registry keeps one theme current, as all editors share the setting. */
    fun colorScheme(context: Context, theme: Theme): TextMateColorScheme {
        ensure(context)
        val registry = ThemeRegistry.getInstance()
        val source = IThemeSource.fromInputStream(textMateTheme(theme).byteInputStream(), "${theme.id}.json", null)
        registry.loadTheme(ThemeModel(source, theme.id).apply { isDark = theme.dark })
        registry.setTheme(theme.id)
        return TextMateColorScheme.create(registry)
    }

    /** The language to highlight a file as, from the core's id; plain text without a grammar. */
    fun language(context: Context, id: String?): Language {
        val scope = id?.let { ensure(context)[it] } ?: return EmptyLanguage()
        return runCatching { TextMateLanguage.create(scope, false) }.getOrElse { EmptyLanguage() }
    }
}
