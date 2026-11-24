@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import java.io.File
import java.nio.file.Files
import java.util.Properties
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

/** Real native core. A document held by the caller, with each stroke it saves
 * added to it and nothing read, must stay exactly the document a complete
 * read returns; and whenever a save does not directly follow what is held,
 * the caller must be told to read instead. */
class AddedStrokeTest {
    private fun fixture(): Properties = Properties().apply {
        val stream = AddedStrokeTest::class.java.classLoader!!.getResourceAsStream("ffi-golden.properties")
            ?: error("Rust FFI golden is missing; run the reviewed fixture generator")
        stream.use { load(it) }
    }
    private fun String.bytes(): ByteArray = chunked(2).map { it.toInt(16).toByte() }.toByteArray()
    private fun UIntArray.toLongArray(): LongArray = LongArray(size) { this[it].toLong() }

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

    @Test fun strokesAddedToAHeldDocumentEqualACompleteReadThroughEditsUndoAndLayers(): Unit = runBlocking {
        val f = fixture(); val core = workbenchCore()
        val directory = Files.createTempDirectory("vw-ffi-smoke-").toFile()
        val now = f.getProperty("time_ms").toLong(); val device = f.getProperty("device_id")
        val documentId = f.getProperty("id2"); val layerId = f.getProperty("id3")
        val project = core.create(CreateProject(File(directory, "project").absolutePath, f.getProperty("id1"), documentId, layerId,
            device, "Added strokes", f.getProperty("source_png_hex").bytes(), now))
        try {
            val changes = project as WorkbenchDocumentChanges
            val added = project as WorkbenchAddedStrokes
            val deferred = project as WorkbenchDeferredStrokes
            var clock = now.toULong()
            fun id(): String = core.newId(++clock)
            suspend fun lamport(): ULong = project.info().nextLamport
            var shift = 0.0
            /** One stroke, saved the way the editor saves it. */
            suspend fun stroke(family: String = "pen"): Pair<String, StrokeSave> {
                val objectId = id(); shift += 3.0
                val options = StrokeOptions(id(), id(), objectId, documentId, layerId, device, lamport(), now + 1, family, 4.0, 0x203040ffu)
                val native = deferred.beginStrokeDeferred(options) ?: project.beginStroke(options)
                try {
                    native.append(SampleBatch(1uL, doubleArrayOf(10.0 + shift, 20.0 + shift, 30.0 + shift), doubleArrayOf(12.0, 24.0, 18.0),
                        uintArrayOf(0u, 8u, 16u), floatArrayOf(0.5f, 0.75f, 1f)))
                    return objectId to (native as WorkbenchStrokeSaves).save()
                } finally { native.dispose() }
            }
            suspend fun edit(vararg commands: EditCommand) { project.edit(EditOptions(id(), documentId, device, lamport(), now + 2), commands.toList()) }
            fun item(snapshot: DocumentSnapshot, objectId: String) = snapshot.render.items.single { it.objectId == objectId }
            var reads = 0
            suspend fun read(held: DocumentSnapshot): DocumentSnapshot { reads++; return checkNotNull(changes.documentChanges(documentId, held)) }

            var held = checkNotNull(changes.documentChanges(documentId, null))
            assertSameDocument(project.document(documentId), held)

            // Stroke after stroke is added to what is held; nothing is read.
            val strokes = mutableListOf<String>()
            repeat(4) {
                val before = project.info()
                val (objectId, saved) = stroke()
                assertEquals(before, checkNotNull(saved.added).previous)
                assertEquals(project.info(), saved.revision)
                val next = checkNotNull(added.withAddedStroke(held, saved))
                assertSameDocument(project.document(documentId), next)
                // Every earlier item is the very instance already held.
                for (earlier in strokes) assertSame(item(held, earlier), item(next, earlier))
                strokes += objectId; held = next
            }
            // The project still knows every stroke it handed out that way: a
            // read of what changed keeps them all.
            var next = read(held)
            assertSameDocument(project.document(documentId), next)
            for (objectId in strokes) assertSame(item(held, objectId), item(next, objectId))
            held = next

            // An edit the caller has not read: the next save does not follow
            // what is held, so nothing is added and the caller reads.
            edit(EditCommand.Create(id(), layerId, Shape.Rectangle(Rect(4.0, 5.0, 20.0, 10.0)), ObjectStyle(0x203040ffu, 2.0)))
            val (objectId, saved) = stroke()
            assertNotNull(saved.added)
            assertNull(added.withAddedStroke(held, saved))
            next = read(held)
            assertSameDocument(project.document(documentId), next)
            for (earlier in strokes) assertSame(item(held, earlier), item(next, earlier))
            strokes += objectId; held = next
            // In step again.
            stroke().let { (addedId, save) ->
                held = checkNotNull(added.withAddedStroke(held, save)); strokes += addedId
                assertSameDocument(project.document(documentId), held)
            }

            // Undo takes that stroke out again; a stroke saved after it does
            // not follow the document that still shows the undone one.
            project.undoRedo(EditOptions(id(), documentId, device, lamport(), now + 3), redo = false)
            val undone = strokes.removeAt(strokes.lastIndex)
            stroke().let { (addedId, save) ->
                assertNull(added.withAddedStroke(held, save))
                held = read(held); strokes += addedId
                assertSameDocument(project.document(documentId), held)
                assertTrue(held.render.items.none { it.objectId == undone })
            }

            // Moving and deleting earlier strokes, read once, then strokes again.
            edit(EditCommand.SetTransform(strokes[0], Transform(e = 7.0, f = -3.0)), EditCommand.Delete(strokes[1]))
            strokes.removeAt(1)
            held = read(held)
            assertSameDocument(project.document(documentId), held)
            repeat(2) {
                val (addedId, save) = stroke()
                held = checkNotNull(added.withAddedStroke(held, save)); strokes += addedId
                assertSameDocument(project.document(documentId), held)
            }

            // Only the snapshot the project returned last is trusted: an equal
            // document read another way is refused, and so is an older one.
            val foreign = project.document(documentId)
            val older = held
            stroke().let { (addedId, save) ->
                assertNull(added.withAddedStroke(foreign, save))
                held = checkNotNull(added.withAddedStroke(held, save)); strokes += addedId
                assertSameDocument(project.document(documentId), held)
                assertNull(added.withAddedStroke(older, save))
                // A save is added once.
                assertNull(added.withAddedStroke(held, save))
            }

            // The first highlighter stroke makes its layer: more than an added
            // stroke. Then a pen stroke lands under the highlighter mark, where
            // a read lists it, and a second highlighter stroke on top.
            val (firstMark, made) = stroke("highlighter")
            assertNull(made.added)
            assertNull(added.withAddedStroke(held, made))
            held = read(held)
            assertSameDocument(project.document(documentId), held)
            assertEquals(firstMark, held.render.items.last().objectId)
            stroke().let { (addedId, save) ->
                held = checkNotNull(added.withAddedStroke(held, save))
                assertSameDocument(project.document(documentId), held)
                assertEquals(listOf(addedId, firstMark), held.render.items.takeLast(2).map { it.objectId })
            }
            stroke("highlighter").let { (secondMark, save) ->
                assertEquals("multiply", checkNotNull(save.added).item.layerBlend)
                held = checkNotNull(added.withAddedStroke(held, save))
                assertSameDocument(project.document(documentId), held)
                assertEquals(secondMark, held.render.items.last().objectId)
            }
            // Three reads in all: the edit, the undo and the new layer, besides
            // the two asked for outright.
            assertEquals(5, reads)
            // After everything, one more read of what changed changes nothing.
            next = read(held)
            assertSameDocument(project.document(documentId), next)
            for (kept in held.render.items.filter { it.shape is Shape.Stroke }) assertSame(kept, item(next, kept.objectId))
        } finally {
            project.close()
            drainNativeTransfers(); CoreSmokeContract.disposeDirectory(directory)
        }
    }

    private fun layer(id: String, visible: Boolean = true, opacity: Double = 1.0) = LayerInfo(id, id, visible, false, opacity, "normal")

    @Test fun anAddedStrokeGoesAfterItsLayerAndTheLayersUnderIt() {
        val layers = listOf(layer("bottom"), layer("middle"), layer("top"))
        val layerOf: (String) -> String = { it.substringBefore(':') }
        // The layer drawn last: at the end, whatever is there.
        assertEquals(0, addedStrokeIndex(layers, emptyList(), "top", layerOf))
        assertEquals(3, addedStrokeIndex(layers, listOf("bottom:a", "middle:b", "top:c"), "top", layerOf))
        // Under other layers: before their items, after its own.
        assertEquals(2, addedStrokeIndex(layers, listOf("bottom:a", "middle:b", "top:c", "top:d"), "middle", layerOf))
        assertEquals(1, addedStrokeIndex(layers, listOf("bottom:a", "top:c"), "bottom", layerOf))
        assertEquals(1, addedStrokeIndex(layers, listOf("bottom:a", "top:c"), "middle", layerOf))
        assertEquals(0, addedStrokeIndex(layers, listOf("middle:b", "top:c"), "bottom", layerOf))
        assertEquals(2, addedStrokeIndex(layers, listOf("bottom:a", "middle:b"), "middle", layerOf))
        // A layer that is unknown or not drawn has no place in the list.
        assertNull(addedStrokeIndex(layers, emptyList(), "elsewhere", layerOf))
        assertNull(addedStrokeIndex(listOf(layer("bottom"), layer("hidden", visible = false)), emptyList(), "hidden", layerOf))
        assertNull(addedStrokeIndex(listOf(layer("clear", opacity = 0.0)), emptyList(), "clear", layerOf))
    }
}
