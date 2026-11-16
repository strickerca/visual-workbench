@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import com.visualworkbench.bindings.core.Cancellation
import java.io.File
import java.nio.file.Files
import java.util.Properties
import kotlin.math.cos
import kotlin.math.sin
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

/** Real native core. The document the apps read now carries packed outlines;
 * they hold exactly the vertices the plain read's lists carry, for real strokes
 * drawn with the phone's default brushes. */
class PackedContoursNativeTest {
    private fun fixture(): Properties = Properties().apply {
        val stream = PackedContoursNativeTest::class.java.classLoader!!.getResourceAsStream("ffi-golden.properties")
            ?: error("Rust FFI golden is missing; run the reviewed fixture generator")
        stream.use { load(it) }
    }
    private fun String.bytes(): ByteArray = chunked(2).map { it.toInt(16).toByte() }.toByteArray()
    private fun visited(contours: Contours): List<Long> = buildList {
        contours.visit(object : ContourVisitor {
            override fun begin(vertexCount: Int) { add(-1L - vertexCount) }
            override fun vertex(x: Long, y: Long) { add(x); add(y) }
            override fun end() { add(Long.MIN_VALUE) }
        })
    }

    @Test fun theDocumentReadCarriesExactlyThePlainReadsOutlines(): Unit = runBlocking {
        val f = fixture(); val core = workbenchCore()
        val directory = Files.createTempDirectory("vw-ffi-smoke-").toFile()
        val now = f.getProperty("time_ms").toLong(); val device = f.getProperty("device_id")
        val documentId = f.getProperty("id2"); val layerId = f.getProperty("id3")
        val project = core.create(CreateProject(File(directory, "project").absolutePath, f.getProperty("id1"), documentId, layerId,
            device, "Packed outlines", f.getProperty("source_png_hex").bytes(), now))
        try {
            var clock = now.toULong()
            fun id(): String = core.newId(++clock)
            val curve = listOf(0.0, 0.0, 0.25, 0.25, 0.75, 0.75, 1.0, 1.0)
            suspend fun stroke(number: Int, samples: Int, family: String, width: Double, stabilization: Float) {
                val native = project.beginStroke(StrokeOptions(id(), id(), id(), documentId, layerId, device, project.info().nextLamport,
                    now + 1, family, width, 0x51c7c2ffu, stabilization, curve))
                try {
                    val x0 = 40.0 * number; val y0 = 30.0 * number
                    native.append(SampleBatch(1uL, DoubleArray(samples) { x0 + 1.4 * it + 5 * sin(.45 * it) },
                        DoubleArray(samples) { y0 + 9 * sin(.31 * it) + 3 * cos(.9 * it) }, UIntArray(samples) { (4 * it).toUInt() },
                        FloatArray(samples) { (.55 + .3 * sin(.2 * it)).toFloat() }))
                    native.commit()
                } finally { native.dispose() }
            }
            stroke(0, 100, "pen", 6.0, .15f); stroke(1, 300, "pen", 6.0, .15f); stroke(2, 3, "pen", 6.0, .15f)
            stroke(3, 100, "highlighter", 24.0, .08f); stroke(4, 100, "marker", 12.0, .10f)
            project.edit(EditOptions(id(), documentId, device, project.info().nextLamport, now + 2),
                listOf(EditCommand.Create(id(), layerId, Shape.Rectangle(Rect(4.0, 5.0, 20.0, 10.0)), ObjectStyle(0x203040ffu, 2.0))))

            val cancellation = Cancellation()
            val plain = try { nativeProjectHandle(project).documentSnapshot(documentId, cancellation) } finally { cancellation.destroy() }
            val read = project.document(documentId)
            val changes = checkNotNull((project as WorkbenchDocumentChanges).documentChanges(documentId, null))
            assertEquals(6, read.render.items.size)
            for ((index, item) in read.render.items.withIndex()) {
                val lists = plain.render.items[index]
                assertEquals(lists.objectId, item.objectId)
                val x = lists.strokeContours.x.toLongArray(); val y = lists.strokeContours.y.toLongArray()
                val ends = lists.strokeContours.ends.toUIntArray()
                assertTrue(item.contours.isPacked)
                assertEquals(x.size, item.contours.vertexCount); assertEquals(ends.size, item.contours.contourCount)
                assertEquals(x.firstOrNull() ?: 0L, item.contours.firstX); assertEquals(y.firstOrNull() ?: 0L, item.contours.firstY)
                // Visited without unpacking: the very values the lists carry.
                assertEquals(visited(Contours(x, y, ends)), visited(item.contours))
                assertArrayEquals(x, item.contours.x); assertArrayEquals(y, item.contours.y)
                assertArrayEquals(ends.toIntArray(), item.contours.ends.toIntArray())
                assertEquals(visited(item.contours), visited(changes.render.items[index].contours))
                if (item.shape is Shape.Stroke) assertTrue(x.size >= 30) else assertEquals(0, x.size)
            }
        } finally {
            project.close()
            drainNativeTransfers(); CoreSmokeContract.disposeDirectory(directory)
        }
    }
}
