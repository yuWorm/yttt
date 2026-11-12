package dev.yttt.spike

import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PairingScanTest {
    /** A QR code as a camera's luminance plane: dark modules 0, light 255. */
    private fun luminance(text: String, size: Int = 240): ByteArray {
        val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, size, size)
        return ByteArray(size * size) { index ->
            if (matrix.get(index % size, index / size)) 0 else 255.toByte()
        }
    }

    @Test
    fun findsThePairingCode() {
        val code = "YTTT-PAIR:1:ABCDEF234567"
        assertEquals(code, decodePairingCode(luminance(code), 240, 240))
    }

    @Test
    fun ignoresOtherCodesAndPlainFrames() {
        assertNull(decodePairingCode(luminance("https://example.com"), 240, 240))
        assertNull(decodePairingCode(ByteArray(240 * 240) { 255.toByte() }, 240, 240))
    }
}
