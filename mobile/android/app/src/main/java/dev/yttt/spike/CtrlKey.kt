package dev.yttt.spike

/**
 * The key bar's Ctrl: armed by a tap, it turns the next key typed on the
 * soft keyboard into its control code, as a terminal keyboard would, and
 * then lets go. Tapping it again before typing cancels it.
 */
class CtrlKey {
    var armed: Boolean = false
        private set

    fun toggle() {
        armed = !armed
    }

    fun apply(typed: ByteArray): ByteArray {
        if (!armed) return typed
        armed = false
        val byte = typed.singleOrNull()?.toInt() ?: return typed
        val code = when (byte) {
            ' '.code -> 0
            in 'a'.code..'z'.code -> byte - 'a'.code + 1
            in '@'.code..'_'.code -> byte - '@'.code
            else -> return typed
        }
        return byteArrayOf(code.toByte())
    }
}
