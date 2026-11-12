package dev.yttt.spike

import uniffi.yttt_mobile_core.FileEntry
import uniffi.yttt_mobile_core.FileKind

data class FileRow(val entry: FileEntry, val depth: Int)

val FileEntry.isDirectory: Boolean
    get() = kind == FileKind.DIRECTORY || kind == FileKind.SYMLINK_DIRECTORY

/**
 * The right drawer's file tree, read a directory at a time: directories
 * first, then by name ignoring case; a directory's entries show while it is
 * expanded. Not thread-safe: use from the UI thread.
 */
class FileTree {
    private val children = HashMap<String, List<FileEntry>>()
    private val expanded = HashSet<String>()

    fun loaded(directory: String, entries: List<FileEntry>) {
        children[directory] = entries.sortedWith(
            compareBy<FileEntry> { !it.isDirectory }.thenBy { it.name.lowercase() }.thenBy { it.name },
        )
    }

    /** Expands or collapses `path`; returns it when its entries must be read first. */
    fun toggle(path: String): String? {
        if (!expanded.add(path)) {
            expanded.remove(path)
            return null
        }
        return path.takeIf { it !in children }
    }

    fun isExpanded(path: String): Boolean = path in expanded

    fun rows(): List<FileRow> {
        val rows = mutableListOf<FileRow>()
        fun add(directory: String, depth: Int) {
            for (entry in children[directory].orEmpty()) {
                rows += FileRow(entry, depth)
                if (entry.isDirectory && entry.path in expanded) add(entry.path, depth + 1)
            }
        }
        add("", 0)
        return rows
    }
}
