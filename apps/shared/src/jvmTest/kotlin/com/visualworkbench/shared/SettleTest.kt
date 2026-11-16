@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import java.io.File
import java.nio.file.Files
import java.util.Properties
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

/** Real native core. Settling, which the phone asks for as it goes to the
 * background, writes nothing for strokes alone, nothing while a stroke is
 * being drawn, and nothing after closing. (The Rust tests write one.) */
class SettleTest {
    private fun fixture(): Properties = Properties().apply {
        val stream = SettleTest::class.java.classLoader!!.getResourceAsStream("ffi-golden.properties")
            ?: error("Rust FFI golden is missing; run the reviewed fixture generator")
        stream.use { load(it) }
    }
    private fun String.bytes(): ByteArray = chunked(2).map { it.toInt(16).toByte() }.toByteArray()

    @Test fun settlingWritesNothingForStrokesWhileDrawingOrAfterClosing(): Unit = runBlocking {
        val f = fixture(); val core = workbenchCore()
        val directory = Files.createTempDirectory("vw-ffi-settle-").toFile()
        val now = f.getProperty("time_ms").toLong(); val device = f.getProperty("device_id")
        val documentId = f.getProperty("id2"); val layerId = f.getProperty("id3")
        try {
            val project = core.create(CreateProject(File(directory, "project").absolutePath, f.getProperty("id1"), documentId, layerId,
                device, "Settle", f.getProperty("source_png_hex").bytes(), now))
            val settling = project as WorkbenchSettling
            var clock = now.toULong()
            fun id(): String = core.newId(++clock)
            val samples = SampleBatch(1uL, doubleArrayOf(10.0, 20.0, 30.0), doubleArrayOf(12.0, 24.0, 18.0), uintArrayOf(0u, 8u, 16u), floatArrayOf(0.5f, 0.75f, 1f))
            val stroke = project.beginStroke(StrokeOptions(id(), id(), id(), documentId, layerId, device, 1uL, now + 1, "pen", 4.0, 0x203040ffu))
            try {
                stroke.append(samples)
                val drawing = assertThrows(CoreFailure::class.java) { runBlocking { settling.settle() } }
                assertEquals(CoreFailureKind.Backpressure, drawing.kind)
                assertEquals(1uL, stroke.commit().hostSeq)
            } finally { stroke.dispose() }
            assertFalse(settling.settle())
            project.close()
            val closed = assertThrows(CoreFailure::class.java) { runBlocking { settling.settle() } }
            assertEquals(CoreFailureKind.Closed, closed.kind)
        } finally { directory.deleteRecursively() }
    }
}
