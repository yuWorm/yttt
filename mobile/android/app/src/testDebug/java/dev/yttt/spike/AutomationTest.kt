package dev.yttt.spike

import org.junit.Assert.assertEquals
import org.junit.Test

class AutomationTest {
    private fun hooks(extras: Map<String, String>, flags: Set<String> = emptySet()) =
        launchHooks(extras::get, { it in flags }, decodeImage = { name -> "decoded:$name" })

    @Test
    fun readsTheLaunchExtras() {
        assertEquals(
            LaunchHooks(pair = "YTTT-PAIR:1:x", type = "ls", relayOnly = true),
            hooks(mapOf("pair" to "YTTT-PAIR:1:x", "type" to "ls"), setOf("relayOnly")),
        )
    }

    @Test
    fun pairsFromAQrImageWithoutACode() {
        assertEquals(LaunchHooks(pair = "decoded:qr.png"), hooks(mapOf("qrImage" to "qr.png")))
        assertEquals(
            LaunchHooks(pair = "code"),
            hooks(mapOf("pair" to "code", "qrImage" to "qr.png")),
        )
    }

    @Test
    fun nothingAskedIsEmpty() {
        assertEquals(LaunchHooks(), hooks(emptyMap()))
    }
}
