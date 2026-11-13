package com.yttt.yai

import org.junit.Assert.assertEquals
import org.junit.Test

class RoutesTest {
    @Test
    fun aProjectPlaceGoesThroughItsRouteUnchanged() {
        for (place in listOf(
            Place("web", "dev", "logs"),
            Place("my project/with spaces", "tab:1?x=&y", "pane#2"),
            Place("web", null, null),
        )) {
            val route = Routes.project(place)
            assertEquals(Routes.PROJECT, route.substringBefore('?'))
            assertEquals(place, Routes.place(Routes.arguments(route)))
        }
    }

    @Test
    fun aFileGoesThroughItsRouteUnchanged() {
        val route = Routes.editor("web", "src/a b/ü&c.ts")
        assertEquals(Routes.EDITOR, route.substringBefore('?'))
        assertEquals("web" to "src/a b/ü&c.ts", Routes.file(Routes.arguments(route)))
    }

    @Test
    fun aTerminalGoesThroughItsRouteUnchanged() {
        val route = Routes.terminal("web:dev:server 1")
        assertEquals("web:dev:server 1", Routes.sessionId(Routes.arguments(route)))
    }
}
