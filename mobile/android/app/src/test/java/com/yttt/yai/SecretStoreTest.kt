package com.yttt.yai

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import javax.crypto.KeyGenerator

class SecretStoreTest {
    @get:Rule val folder = TemporaryFolder()

    private val key = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
    private fun store() = SecretStore(folder.root) { key }

    @Test
    fun aNewSecretIsMadeOnceKeptEncryptedAndReadBack() {
        val first = store().deviceSecret()
        assertEquals(32, first.size)
        assertArrayEquals(first, store().deviceSecret())
        val stored = folder.root.resolve("device-secret.enc").readBytes()
        assertEquals(12 + 32 + 16, stored.size)
        // Not the secret in the clear.
        assertFalse(stored.toList().windowed(32).any { it == first.toList() })
    }

    @Test
    fun aTamperedFileIsNotTakenForTheSecret() {
        val first = store().deviceSecret()
        val file = folder.root.resolve("device-secret.enc")
        val bytes = file.readBytes()
        bytes[20] = (bytes[20].toInt() xor 1).toByte()
        file.writeBytes(bytes)
        val result = runCatching { store().deviceSecret() }
        assertTrue(result.isFailure || !result.getOrThrow().contentEquals(first))
        assertNotEquals(null, result.exceptionOrNull())
    }
}
