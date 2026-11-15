@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import java.io.File
import java.nio.file.Files
import java.util.Properties
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

/** Real native core. Reading a document as changes must give exactly what the
 * plain read gives, while unchanged strokes stay the caller's own instances. */
class DocumentChangesTest {
    private fun fixture(): Properties = Properties().apply {
        val stream = DocumentChangesTest::class.java.classLoader!!.getResourceAsStream("ffi-golden.properties")
            ?: error("Rust FFI golden is missing; run the reviewed fixture generator")
        stream.use { load(it) }
    }
    private fun String.bytes(): ByteArray = chunked(2).map { it.toInt(16).toByte() }.toByteArray()

    /** Field-for-field equality; the data classes compare their arrays by identity. */
    private fun assertSameDocument(expected: DocumentSnapshot, actual: DocumentSnapshot) {
        assertEquals(expected.copy(render = RenderList(expected.render.revision, emptyList())),
            actual.copy(render = RenderList(actual.render.revision, emptyList())))
        assertEquals(expected.render.items.map { it.objectId }, actual.render.items.map { it.objectId })
        for ((a, b) in expected.render.items.zip(actual.render.items)) {
            assertArrayEquals(a.contours.x, b.contours.x); assertArrayEquals(a.contours.y, b.contours.y)
            assertArrayEquals(a.contours.ends.toLongArray(), b.contours.ends.toLongArray())
            val empty = Contours(longArrayOf(), longArrayOf(), uintArrayOf())
            assertEquals(a.copy(contours = empty), b.copy(contours = empty))
        }
    }
    private fun UIntArray.toLongArray(): LongArray = LongArray(size) { this[it].toLong() }

    @Test fun changesEqualThePlainReadAndKeepUnchangedStrokeInstances(): Unit = runBlocking {
        val f = fixture(); val core = workbenchCore()
        val directory = Files.createTempDirectory("vw-ffi-smoke-").toFile()
        val now = f.getProperty("time_ms").toLong(); val device = f.getProperty("device_id")
        val documentId = f.getProperty("id2"); val layerId = f.getProperty("id3")
        val project = core.create(CreateProject(File(directory, "project").absolutePath, f.getProperty("id1"), documentId, layerId,
            device, "Document changes", f.getProperty("source_png_hex").bytes(), now))
        try {
            val changes = project as WorkbenchDocumentChanges
            var clock = now.toULong()
            fun id(): String = core.newId(++clock)
            suspend fun lamport(): ULong = project.info().nextLamport
            suspend fun stroke(shift: Double): String {
                val objectId = id()
                val native = project.beginStroke(StrokeOptions(id(), id(), objectId, documentId, layerId, device, lamport(), now + 1, "pen", 4.0, 0x203040ffu))
                try {
                    native.append(SampleBatch(1uL, doubleArrayOf(10.0 + shift, 20.0 + shift, 30.0 + shift), doubleArrayOf(12.0, 24.0, 18.0),
                        uintArrayOf(0u, 8u, 16u), floatArrayOf(0.5f, 0.75f, 1f)))
                    native.commit()
                } finally { native.dispose() }
                return objectId
            }
            suspend fun edit(vararg commands: EditCommand) { project.edit(EditOptions(id(), documentId, device, lamport(), now + 2), commands.toList()) }

            // First read: no earlier snapshot. Same content as the plain read.
            var held = checkNotNull(changes.documentChanges(documentId, null))
            assertSameDocument(project.document(documentId), held)
            assertTrue(held.render.items.isEmpty())

            val first = stroke(0.0); val second = stroke(3.0)
            edit(EditCommand.Create(id(), layerId, Shape.Rectangle(Rect(4.0, 5.0, 20.0, 10.0)), ObjectStyle(0x203040ffu, 2.0)))
            var next = checkNotNull(changes.documentChanges(documentId, held))
            assertSameDocument(project.document(documentId), next)
            assertEquals(3, next.render.items.size)
            held = next

            // A third stroke: the two earlier strokes are the very same instances.
            val third = stroke(6.0)
            next = checkNotNull(changes.documentChanges(documentId, held))
            assertSameDocument(project.document(documentId), next)
            fun item(snapshot: DocumentSnapshot, objectId: String) = snapshot.render.items.single { it.objectId == objectId }
            assertSame(item(held, first), item(next, first)); assertSame(item(held, second), item(next, second))
            assertEquals(4, next.render.items.size)
            held = next

            // Moving one stroke rebuilds that stroke only.
            edit(EditCommand.SetTransform(second, Transform(e = 7.0, f = -3.0)))
            next = checkNotNull(changes.documentChanges(documentId, held))
            assertSameDocument(project.document(documentId), next)
            assertSame(item(held, first), item(next, first)); assertSame(item(held, third), item(next, third))
            assertNotSame(item(held, second), item(next, second))
            assertEquals(7.0, item(next, second).transform.e, 0.0)
            held = next

            // Delete, then undo: still the same document as the plain read.
            edit(EditCommand.Delete(first))
            next = checkNotNull(changes.documentChanges(documentId, held))
            assertSameDocument(project.document(documentId), next)
            assertTrue(next.render.items.none { it.objectId == first })
            held = next
            project.undoRedo(EditOptions(id(), documentId, device, lamport(), now + 3), redo = false)
            next = checkNotNull(changes.documentChanges(documentId, held))
            assertSameDocument(project.document(documentId), next)
            assertTrue(next.render.items.any { it.objectId == first })
            assertSame(item(held, third), item(next, third))
            held = next

            // A snapshot that is not the one returned last is never trusted:
            // its strokes are simply read again, and the result is still right.
            val foreign = project.document(documentId)
            next = checkNotNull(changes.documentChanges(documentId, foreign))
            assertSameDocument(foreign, next)
            assertNotSame(item(foreign, third), item(next, third))
            // An unchanged document read twice keeps every stroke instance.
            val again = checkNotNull(changes.documentChanges(documentId, next))
            for (objectId in listOf(first, second, third)) assertSame(item(next, objectId), item(again, objectId))
            assertSameDocument(project.document(documentId), again)
        } finally {
            project.close()
            drainNativeTransfers(); CoreSmokeContract.disposeDirectory(directory)
        }
    }
}
