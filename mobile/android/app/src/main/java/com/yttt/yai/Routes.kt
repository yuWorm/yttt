package com.yttt.yai

import java.net.URLDecoder
import java.net.URLEncoder

/**
 * The pushed screens' routes and their arguments. Values are percent-encoded
 * (a space as %20, not +: Navigation decodes with Uri.decode), and a missing
 * value is left out of the route.
 */
object Routes {
    const val PROJECT = "project"
    const val EDITOR = "editor"
    const val TERMINAL = "terminal"
    const val ARG_PROJECT = "project"
    const val ARG_TAB = "tab"
    const val ARG_PANE = "pane"
    const val ARG_PATH = "path"
    const val ARG_SESSION = "session"
    const val PROJECT_PATTERN = "$PROJECT?$ARG_PROJECT={$ARG_PROJECT}&$ARG_TAB={$ARG_TAB}&$ARG_PANE={$ARG_PANE}"
    const val EDITOR_PATTERN = "$EDITOR?$ARG_PROJECT={$ARG_PROJECT}&$ARG_PATH={$ARG_PATH}"
    const val TERMINAL_PATTERN = "$TERMINAL?$ARG_SESSION={$ARG_SESSION}"

    private fun encode(value: String) = URLEncoder.encode(value, "UTF-8").replace("+", "%20")

    private fun route(base: String, vararg arguments: Pair<String, String?>): String =
        base + "?" + arguments.filter { it.second != null }.joinToString("&") { (key, value) -> key + "=" + encode(value!!) }

    fun project(place: Place): String =
        route(PROJECT, ARG_PROJECT to place.projectId, ARG_TAB to place.tabId, ARG_PANE to place.paneId)

    fun editor(projectId: String, path: String): String = route(EDITOR, ARG_PROJECT to projectId, ARG_PATH to path)

    fun terminal(sessionId: String): String = route(TERMINAL, ARG_SESSION to sessionId)

    /** A route's arguments, decoded, as Navigation hands them to a screen. */
    fun arguments(route: String): Map<String, String?> =
        route.substringAfter('?', "").split('&').filter { it.contains('=') }.associate {
            it.substringBefore('=') to URLDecoder.decode(it.substringAfter('='), "UTF-8")
        }

    fun place(arguments: Map<String, String?>): Place =
        Place(arguments[ARG_PROJECT].orEmpty(), arguments[ARG_TAB], arguments[ARG_PANE])

    fun file(arguments: Map<String, String?>): Pair<String, String> =
        arguments[ARG_PROJECT].orEmpty() to arguments[ARG_PATH].orEmpty()

    fun sessionId(arguments: Map<String, String?>): String = arguments[ARG_SESSION].orEmpty()
}
