package dev.yttt.spike

import org.junit.Assert.assertEquals
import org.junit.Test

/** A release build ignores every launch hook. */
class AutomationTest {
    @Test
    fun ignoresTheLaunchExtras() {
        val extras = mapOf("pair" to "YTTT-PAIR:1:x", "type" to "ls", "qrImage" to "qr.png")
        assertEquals(LaunchHooks(), launchHooks(extras::get, { true }, decodeImage = { "decoded" }))
    }
}
