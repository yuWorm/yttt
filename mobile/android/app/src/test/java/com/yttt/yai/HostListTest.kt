package com.yttt.yai

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class HostListTest {
    @get:Rule val folder = TemporaryFolder()

    private fun bytes(text: String) = text.toByteArray()
    private val a = "aaaaaaaa1111"
    private val b = "bbbbbbbb2222"

    @Test
    fun aNewHostIsAddedAtTheEndAndBecomesCurrent() {
        val list = HostList().added(a, bytes("A")).added(b, bytes("B"))
        assertEquals(listOf(a, b), list.hosts.map { it.id })
        assertEquals(b, list.current)
        assertArrayEquals(bytes("B"), list.currentRecord)
    }

    @Test
    fun pairingAgainReplacesTheRecordInPlace() {
        val list = HostList().added(a, bytes("A")).added(b, bytes("B")).added(a, bytes("A2"))
        assertEquals(listOf(a, b), list.hosts.map { it.id })
        assertEquals(a, list.current)
        assertArrayEquals(bytes("A2"), list.currentRecord)
    }

    @Test
    fun removingTheCurrentHostFallsBackToTheFirstLeft() {
        val list = HostList().added(a, bytes("A")).added(b, bytes("B"))
        assertEquals(a, list.removed(b).current)
        assertEquals(b, list.selected(a).removed(a).current)
        assertEquals(b, list.removed(a).current)
        val empty = list.removed(a).removed(b)
        assertNull(empty.current)
        assertNull(empty.currentRecord)
    }

    @Test
    fun selectingSwitchesOnlyToAKnownHost() {
        val list = HostList().added(a, bytes("A")).added(b, bytes("B"))
        assertEquals(a, list.selected(a).current)
        assertEquals(b, list.selected("unknown").current)
    }

    @Test
    fun hostsWithTheSameNameAreToldApartByTheirIdEnd() {
        val c = "cccccccc3333"
        val list = HostList().added(a, bytes("mac")).added(b, bytes("mac")).added(c, bytes("linux"))
        assertEquals(
            listOf(a to "mac · 1111", b to "mac · 2222", c to "linux"),
            list.names { String(it) },
        )
    }

    @Test
    fun theListIsKept() {
        val store = HostStore(folder.root)
        assertEquals(emptyList<HostEntry>(), store.load().hosts)
        assertNull(store.load().current)

        store.save(HostList().added(a, bytes("old")).added(b, bytes("new")).selected(a))
        val loaded = store.load()
        assertEquals(listOf(a, b), loaded.hosts.map { it.id })
        assertEquals(a, loaded.current)
        assertArrayEquals(bytes("new"), loaded.hosts[1].record)
    }

    @Test
    fun aDamagedListReadsAsNoHostsRatherThanFailing() {
        // As a power cut can leave a file renamed into place before its data reached the disk.
        folder.root.resolve("hosts.json").writeText("")
        assertEquals(emptyList<HostEntry>(), HostStore(folder.root).load().hosts)
        folder.root.resolve("hosts.json").writeText("{\"hosts\": [")
        assertEquals(emptyList<HostEntry>(), HostStore(folder.root).load().hosts)
    }
}
