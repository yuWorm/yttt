package com.yttt.yai

import java.io.File
import java.util.Base64
import org.json.JSONArray
import org.json.JSONObject

/** A paired Host: `id` from the core's `hostInfo`, `record` as the core stores it. */
class HostEntry(val id: String, val record: ByteArray)

/**
 * The Hosts this phone paired with, in the order they were added, and the
 * one it connects to. Values: each change returns a new list.
 */
class HostList(val hosts: List<HostEntry> = emptyList(), val current: String? = null) {
    val currentRecord: ByteArray? get() = hosts.firstOrNull { it.id == current }?.record

    /** A new Host goes last; pairing with a known one again replaces its record in place. */
    fun added(id: String, record: ByteArray): HostList {
        val entry = HostEntry(id, record)
        val replaced = hosts.map { if (it.id == id) entry else it }
        return HostList(if (hosts.any { it.id == id }) replaced else hosts + entry, id)
    }

    /** The current Host falls back to the first one left. */
    fun removed(id: String): HostList {
        val left = hosts.filter { it.id != id }
        return HostList(left, if (current == id) left.firstOrNull()?.id else current)
    }

    fun selected(id: String): HostList =
        if (hosts.any { it.id == id }) HostList(hosts, id) else this

    /** Each Host's name; Hosts with the same name get their id's last four characters. */
    fun names(label: (ByteArray) -> String): List<Pair<String, String>> {
        val labels = hosts.map { it.id to label(it.record) }
        val counts = labels.groupingBy { it.second }.eachCount()
        return labels.map { (id, name) -> id to if (counts.getValue(name) > 1) "$name · ${id.takeLast(4)}" else name }
    }
}

/**
 * The list in `hosts.json`, in the app's private files (backups are off).
 */
class HostStore(private val dir: File) {
    private val file = File(dir, "hosts.json")

    fun load(): HostList = if (file.exists()) decode(file.readBytes()) else HostList()

    fun save(list: HostList) {
        val json = JSONObject()
        list.current?.let { json.put("current", it) }
        json.put("hosts", JSONArray(list.hosts.map {
            JSONObject().put("id", it.id).put("record", Base64.getEncoder().encodeToString(it.record))
        }))
        val partial = File(dir, "hosts.json.partial")
        partial.writeText(json.toString())
        check(partial.renameTo(file)) { "could not store the paired Hosts" }
    }

    private fun decode(bytes: ByteArray): HostList {
        val json = JSONObject(String(bytes))
        val hosts = json.getJSONArray("hosts")
        val entries = (0 until hosts.length()).map {
            val host = hosts.getJSONObject(it)
            HostEntry(host.getString("id"), Base64.getDecoder().decode(host.getString("record")))
        }
        return HostList(entries, json.optString("current").takeIf { it.isNotEmpty() })
    }
}
