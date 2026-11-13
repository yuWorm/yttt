package dev.yttt.spike

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.yttt_mobile_core.FileEntry
import uniffi.yttt_mobile_core.FileKind

class FileTreeTest {
    private fun file(path: String) = FileEntry(path.substringAfterLast('/'), path, FileKind.FILE)
    private fun dir(path: String) = FileEntry(path.substringAfterLast('/'), path, FileKind.DIRECTORY)

    @Test
    fun directoriesComeFirstThenNamesInOrderAndChildrenShowWhenExpanded() {
        val tree = FileTree()
        tree.loaded("", listOf(file("b.txt"), dir("src"), file("A.md"), dir("docs")))
        assertEquals(listOf("docs", "src", "A.md", "b.txt"), tree.rows().map { it.entry.path })
        assertTrue(tree.rows().all { it.depth == 0 })

        // Expanding a directory not read yet asks for it.
        assertEquals("src", tree.toggle("src"))
        tree.loaded("src", listOf(file("src/main.js"), dir("src/lib")))
        assertEquals(
            listOf("docs" to 0, "src" to 0, "src/lib" to 1, "src/main.js" to 1, "A.md" to 0, "b.txt" to 0),
            tree.rows().map { it.entry.path to it.depth },
        )
        // Collapsing hides the children; expanding again needs no read.
        assertEquals(null, tree.toggle("src"))
        assertEquals(listOf("docs", "src", "A.md", "b.txt"), tree.rows().map { it.entry.path })
        assertEquals(null, tree.toggle("src"))
        assertTrue(tree.isExpanded("src"))
        assertEquals(6, tree.rows().size)
    }
}
