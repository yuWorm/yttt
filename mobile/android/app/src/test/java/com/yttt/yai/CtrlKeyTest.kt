package com.yttt.yai

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CtrlKeyTest {
    @Test
    fun anArmedCtrlTurnsTheNextLetterIntoAControlCodeOnce() {
        val ctrl = CtrlKey()
        assertArrayEquals(byteArrayOf('c'.code.toByte()), ctrl.apply("c".toByteArray()))
        ctrl.toggle()
        assertTrue(ctrl.armed)
        assertArrayEquals(byteArrayOf(0x03), ctrl.apply("c".toByteArray()))
        assertFalse(ctrl.armed)
        assertArrayEquals(byteArrayOf('c'.code.toByte()), ctrl.apply("c".toByteArray()))
    }

    @Test
    fun capitalsAndPunctuationMapLikeATerminalKeyboard() {
        val cases = mapOf("C" to 0x03, "a" to 0x01, "z" to 0x1a, "@" to 0x00, "[" to 0x1b, "\\" to 0x1c,
            "]" to 0x1d, "^" to 0x1e, "_" to 0x1f, " " to 0x00)
        for ((typed, code) in cases) {
            val ctrl = CtrlKey().apply { toggle() }
            assertArrayEquals(typed, byteArrayOf(code.toByte()), ctrl.apply(typed.toByteArray()))
        }
    }

    @Test
    fun anythingElseGoesThroughAndTapping_ctrlAgainCancels() {
        val ctrl = CtrlKey().apply { toggle() }
        assertArrayEquals("你".toByteArray(), ctrl.apply("你".toByteArray()))
        assertFalse(ctrl.armed)
        ctrl.toggle()
        ctrl.toggle()
        assertFalse(ctrl.armed)
    }
}
