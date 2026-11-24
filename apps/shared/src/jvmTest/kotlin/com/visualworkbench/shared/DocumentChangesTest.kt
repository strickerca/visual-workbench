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

    /** A stroke begun without reading the project saves like any other, takes
     * ink before an earlier stroke is saved, and a highlighter is handed back
     * to the ordinary begin. */
    @Test fun aDeferredStrokeTakesInkBeforeTheEarlierOneIsSavedAndSavesInOrder(): Unit = runBlocking {
        val f = fixture(); val core = workbenchCore()
        val directory = Files.createTempDirectory("vw-ffi-smoke-").toFile()
        val now = f.getProperty("time_ms").toLong(); val device = f.getProperty("device_id")
        val documentId = f.getProperty("id2"); val layerId = f.getProperty("id3")
        val project = core.create(CreateProject(File(directory, "project").absolutePath, f.getProperty("id1"), documentId, layerId,
            device, "Deferred strokes", f.getProperty("source_png_hex").bytes(), now))
        try {
            val deferred = project as WorkbenchDeferredStrokes
            var clock = now.toULong()
            fun id(): String = core.newId(++clock)
            fun options(objectId: String, lamport: ULong, family: String = "pen") =
                StrokeOptions(id(), id(), objectId, documentId, layerId, device, lamport, now + 1, family, 4.0, 0x203040ffu)
            val samples = SampleBatch(1uL, doubleArrayOf(10.0, 20.0, 30.0), doubleArrayOf(12.0, 24.0, 18.0), uintArrayOf(0u, 8u, 16u), floatArrayOf(0.5f, 0.75f, 1f))
            assertNull(deferred.beginStrokeDeferred(options(id(), 1uL, "highlighter")))
            val firstId = id(); val secondId = id()
            val first = project.beginStroke(options(firstId, 1uL))
            val second = checkNotNull(deferred.beginStrokeDeferred(options(secondId, 4uL)))
            try {
                first.append(samples)
                // The second stroke has ink while the first is not saved yet.
                assertEquals(3uL, second.append(samples).sampleCount)
                assertEquals(1uL, first.commit().hostSeq)
                assertEquals(2uL, second.commit().hostSeq)
            } finally { first.dispose(); second.dispose() }
            assertEquals(listOf(firstId, secondId), project.document(documentId).render.items.map { it.objectId })
        } finally {
            project.close()
            drainNativeTransfers(); CoreSmokeContract.disposeDirectory(directory)
        }
    }

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

    /** Objects that are not strokes come whole at every read. One that did not
     * change is the caller's very instance, as an unchanged stroke is, so that
     * a caller tells every object that changed by identity; one that changed is
     * another instance, and the document is the plain read either way. */
    @Test fun unchangedObjectsOfEveryKindKeepTheCallersInstances(): Unit = runBlocking {
        val f = fixture(); val core = workbenchCore()
        val directory = Files.createTempDirectory("vw-ffi-smoke-").toFile()
        val now = f.getProperty("time_ms").toLong(); val device = f.getProperty("device_id")
        val documentId = f.getProperty("id2"); val layerId = f.getProperty("id3")
        val project = core.create(CreateProject(File(directory, "project").absolutePath, f.getProperty("id1"), documentId, layerId,
            device, "Every kind", f.getProperty("source_png_hex").bytes(), now))
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
            fun item(snapshot: DocumentSnapshot, objectId: String) = snapshot.render.items.single { it.objectId == objectId }

            val pen = stroke(0.0)
            val box = id(); val arrow = id(); val text = id()
            edit(EditCommand.Create(box, layerId, Shape.Rectangle(Rect(4.0, 5.0, 20.0, 10.0)), ObjectStyle(0x203040ffu, 2.0, fill = 0x8ecae6ffu)),
                EditCommand.Create(arrow, layerId, Shape.Arrow(listOf(Point(2.0, 3.0), Point(40.0, 30.0))), ObjectStyle(0xc0392bffu, 3.0)),
                EditCommand.Create(text, layerId, Shape.Text(Point(6.0, 40.0), "Label", "Inter", 14.0), ObjectStyle(0x1f7a4dffu, 1.0)))
            // A numbered marker, placed as the instruction list places one.
            val marker = id()
            run {
                val info = project.info()
                val plan = instructionWorkflows(project).prepare(WorkflowBinding(info.projectId, documentId, info.hostSeq, info.stateHash),
                    WorkflowMetadata(id(), device, info.nextLamport, now + 3), InstructionCommand.PlaceMarker(marker, id(), layerId, Point(30.0, 20.0), null,
                        emptyList(), ObjectStyle(0x2456c8ffu, 2.0), InstructionRole.Change, "Check this", InstructionEntryMethod.PhoneKeyboard))
                try { plan.commit() } finally { plan.close() }
            }
            var held = checkNotNull(changes.documentChanges(documentId, null))
            assertSameDocument(project.document(documentId), held)
            val others = listOf(box, arrow, text, marker)
            assertTrue(item(held, marker).shape is Shape.Marker)

            // A stroke added, then one moved: every other object is the instance held.
            val added = stroke(5.0)
            var next = checkNotNull(changes.documentChanges(documentId, held))
            assertSameDocument(project.document(documentId), next)
            for (objectId in others + pen) assertSame(objectId, item(held, objectId), item(next, objectId))
            held = next
            edit(EditCommand.SetTransform(added, Transform(e = 4.0, f = 2.0)))
            next = checkNotNull(changes.documentChanges(documentId, held))
            assertSameDocument(project.document(documentId), next)
            for (objectId in others + pen) assertSame(objectId, item(held, objectId), item(next, objectId))
            assertNotSame(item(held, added), item(next, added))
            held = next

            // The rectangle restyled: it alone is another instance.
            edit(EditCommand.SetStyle(box, ObjectStyle(0xe08a00ffu, 2.0, fill = 0x8ecae6ffu)))
            next = checkNotNull(changes.documentChanges(documentId, held))
            assertSameDocument(project.document(documentId), next)
            assertNotSame(item(held, box), item(next, box))
            assertEquals(0xe08a00ffu, item(next, box).style.rgba)
            for (objectId in listOf(arrow, text, marker, pen, added)) assertSame(objectId, item(held, objectId), item(next, objectId))
            held = next

            // The marker moved: it alone is another instance.
            val placed = item(held, marker).transform
            edit(EditCommand.SetTransform(marker, Transform(e = 9.0, f = 1.0)))
            next = checkNotNull(changes.documentChanges(documentId, held))
            assertSameDocument(project.document(documentId), next)
            assertNotSame(item(held, marker), item(next, marker))
            for (objectId in listOf(box, arrow, text, pen, added)) assertSame(objectId, item(held, objectId), item(next, objectId))
            held = next

            // Undo of the move: the marker as it was, in yet another instance, and the rest kept.
            project.undoRedo(EditOptions(id(), documentId, device, lamport(), now + 4), redo = false)
            next = checkNotNull(changes.documentChanges(documentId, held))
            assertSameDocument(project.document(documentId), next)
            assertNotSame(item(held, marker), item(next, marker))
            assertEquals(placed, item(next, marker).transform)
            for (objectId in listOf(box, arrow, text, pen, added)) assertSame(objectId, item(held, objectId), item(next, objectId))
        } finally {
            project.close()
            drainNativeTransfers(); CoreSmokeContract.disposeDirectory(directory)
        }
    }
}
