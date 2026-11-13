package com.yttt.yai

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import javax.crypto.KeyGenerator

class AccountStoreTest {
    @get:Rule val folder = TemporaryFolder()

    private val key = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
    private fun store() = AccountStore(folder.root) { key }
    private val account = RelayAccount(
        account = "{\"session\":{\"token\":\"secret-token\"}}".toByteArray(),
        email = "owner@example.com",
        controlUrl = "https://control.example.com",
    )

    @Test
    fun anAccountIsKeptEncryptedAndReadBack() {
        assertNull(store().load())
        store().save(account)
        val loaded = store().load()!!
        assertArrayEquals(account.account, loaded.account)
        assertEquals("owner@example.com", loaded.email)
        assertEquals("https://control.example.com", loaded.controlUrl)
        assertEquals("control.example.com", loaded.controlHost)
        val stored = folder.root.resolve("relay-account.enc").readBytes().toString(Charsets.ISO_8859_1)
        assertFalse(stored.contains("secret-token"))
        assertFalse(stored.contains("owner@example.com"))
    }

    @Test
    fun signingOutForgetsIt() {
        store().save(account)
        store().delete()
        assertNull(store().load())
        assertFalse(folder.root.resolve("relay-account.enc").exists())
    }

    @Test
    fun aDamagedFileIsNoAccount() {
        store().save(account)
        val file = folder.root.resolve("relay-account.enc")
        file.writeBytes(file.readBytes().also { it[20] = (it[20].toInt() xor 1).toByte() })
        assertNull(store().load())
        file.writeBytes(ByteArray(0))
        assertNull(store().load())
    }
}
