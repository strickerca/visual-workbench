@file:OptIn(ExperimentalUnsignedTypes::class)

package com.visualworkbench.android

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.*
import com.visualworkbench.android.ui.WorkbenchPalette
import com.visualworkbench.shared.*
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.UUID
import kotlin.math.abs

/** Native JNI, durable project and Android path rendering; no physical-pen claims. */
@RunWith(AndroidJUnit4::class)
class EditorCoreInstrumentedTest {
    private class Fixture {
        val core = workbenchCore()
        val directory = File(InstrumentationRegistry.getInstrumentation().targetContext.cacheDir, "editor-fixture-${UUID.randomUUID()}")
        val device = core.newDeviceId()
        val projectId = id(); val documentId = id(); val layerId = id()
        var counter = 1uL
        fun id() = core.newId(System.currentTimeMillis().toULong())
        fun options(count: Int = 1): EditOptions = EditOptions(id(), documentId, device, counter, System.currentTimeMillis()).also { counter += count.toUInt() }
        suspend fun create(width: Int = 480, height: Int = 240): WorkbenchProject {
            val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
            val source = try { bitmap.eraseColor(android.graphics.Color.WHITE); ByteArrayOutputStream().also { assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }.toByteArray() }
            finally { bitmap.recycle() }
            return core.create(CreateProject(directory.absolutePath, projectId, documentId, layerId, device, "Synthetic editor fixture", source, System.currentTimeMillis()))
        }
        fun clean() {
            val root = InstrumentationRegistry.getInstrumentation().targetContext.cacheDir.canonicalFile
            val target = directory.canonicalFile
            assertEquals(root, target.parentFile)
            assertTrue(target.name.startsWith("editor-fixture-"))
            assertTrue(!target.exists() || target.deleteRecursively())
        }
    }

    @Test fun typedToolsAndTextRemainEditableAfterCloseAndReopen() = runBlocking {
        val fixture = Fixture(); var project: WorkbenchProject? = null
        try {
            project = fixture.create()
            val shapes = listOf<Shape>(Shape.Line(listOf(Point(10.0, 10.0), Point(80.0, 20.0))),
                Shape.Arrow(listOf(Point(10.0, 40.0), Point(80.0, 50.0))), Shape.Rectangle(Rect(110.0, 10.0, 60.0, 40.0)),
                Shape.Ellipse(Rect(190.0, 10.0, 60.0, 40.0)), Shape.Text(Point(10.0, 100.0), "Editable ffi", "Inter", 24.0))
            val ids = shapes.map { fixture.id() }
            project.edit(fixture.options(shapes.size), shapes.mapIndexed { index, shape -> EditCommand.Create(ids[index], fixture.layerId, shape, ObjectStyle(0x172b3affu, 3.0)) })
            val moved = Transform(e = 12.5, f = 3.25)
            project.edit(fixture.options(), listOf(EditCommand.SetTransform(ids[0], moved)))
            project.edit(fixture.options(3), listOf(EditCommand.SetText(ids.last(), "Still the same object", "Noto Sans", 28.0)))
            project.edit(fixture.options(2), listOf(EditCommand.SetStyle(ids[2], ObjectStyle(0xff3311ffu, 9.0))))
            val before = project.document(fixture.documentId)
            assertEquals(5, before.render.items.size)
            assertEquals(moved, before.render.items.first { it.objectId == ids[0] }.transform)
            assertEquals("Still the same object", (before.render.items.first { it.objectId == ids.last() }.shape as Shape.Text).text)
            val hash = project.info().stateHash
            project.close(); project = fixture.core.open(fixture.directory.absolutePath)
            assertEquals(hash, project.info().stateHash)
            assertTrue(project.info().canUndo)
            project.undoRedo(fixture.options(512), false)
            assertEquals(3.0, project.document(fixture.documentId).render.items.first { it.objectId == ids[2] }.style.width, .0)
            project.undoRedo(fixture.options(512), true)
            assertEquals(hash, project.info().stateHash)
        } finally { project?.close(); fixture.clean() }
    }

    @Test fun realCorePressureCurveStabilizationAndPredictionHaveDistinctSemantics() = runBlocking {
        val fixture = Fixture(); var project: WorkbenchProject? = null
        try {
            project = fixture.create()
            val current = project
            suspend fun contours(curve: List<Double>, stabilization: Float): Contours {
                val native = current.beginStroke(StrokeOptions(fixture.id(), fixture.id(), fixture.id(), fixture.documentId,
                    fixture.layerId, fixture.device, fixture.counter, System.currentTimeMillis(), "pen", 16.0, 0x172b3affu, stabilization, curve))
                fixture.counter += 3u
                try {
                    val count = 48
                    val samples = SampleBatch(1uL, DoubleArray(count) { 12.0 + it * 4 }, DoubleArray(count) { 60.0 + if (it % 2 == 0) 12.0 else -12.0 },
                        UIntArray(count) { (it * 8).toUInt() }, FloatArray(count) { .05f + .9f * it / (count - 1) })
                    val real = native.append(samples)
                    val prediction = native.predict(SampleBatch(2uL, doubleArrayOf(220.0), doubleArrayOf(100.0), uintArrayOf(400u), floatArrayOf(.8f)))
                    assertTrue(prediction.contours.ends.isNotEmpty())
                    // Exact append retry still returns canonical real geometry:
                    // prediction never advances sequence or stored sample count.
                    val retried = native.append(samples)
                    assertArrayEquals(real.contours.x, retried.contours.x)
                    assertArrayEquals(real.contours.y, retried.contours.y)
                    assertEquals(real.sampleCount, retried.sampleCount)
                    return real.contours
                } finally { native.cancel(); native.dispose() }
            }
            val linear = listOf(0.0, 0.0, .25, .25, .75, .75, 1.0, 1.0)
            val soft = listOf(0.0, 0.0, .2, .05, .8, .3, 1.0, 1.0)
            val a = contours(linear, 0f); val b = contours(soft, 0f); val c = contours(linear, 1f)
            assertFalse(a.x.contentEquals(b.x) && a.y.contentEquals(b.y))
            assertFalse(a.x.contentEquals(c.x) && a.y.contentEquals(c.y))
            assertEquals(0, project.document(fixture.documentId).render.items.size)
        } finally { project?.close(); fixture.clean() }
    }

    @Test fun highlighterIsMultiplyAndStrokeWidthEditChangesGeometryWithUndo() = runBlocking {
        val fixture = Fixture(); var project: WorkbenchProject? = null
        try {
            project = fixture.create()
            val id = fixture.id()
            val stroke = project.beginStroke(StrokeOptions(fixture.id(), fixture.id(), id, fixture.documentId, fixture.layerId,
                fixture.device, fixture.counter, System.currentTimeMillis(), "highlighter", 12.0, 0xffcf5766u))
            fixture.counter += 3u
            try {
                stroke.append(SampleBatch(1uL, doubleArrayOf(20.0, 120.0), doubleArrayOf(40.0, 40.0), uintArrayOf(0u, 16u), floatArrayOf(.5f, .8f)))
                stroke.commit()
            } finally { stroke.dispose() }
            val before = project.document(fixture.documentId).render.items.single()
            assertEquals("multiply", before.layerBlend)
            project.edit(fixture.options(2), listOf(EditCommand.SetStyle(id, before.style.copy(width = 32.0))))
            val after = project.document(fixture.documentId).render.items.single()
            assertFalse(before.contours.y.contentEquals(after.contours.y))
            project.undoRedo(fixture.options(512), false)
            assertArrayEquals(before.contours.y, project.document(fixture.documentId).render.items.single().contours.y)
        } finally { project?.close(); fixture.clean() }
    }

    @Test fun geometryHitTestsDoNotEraseAnEmptyEllipseCenterAndSweepCatchesThinLines() {
        val info = ProjectInfo("fixture", "Fixture", "fixture-device", 1uL, false, false, 0uL, "", listOf("document"))
        val style = ObjectStyle(0x172b3affu, 2.0)
        fun objectFor(id: String, shape: Shape) = EditorDrawing.objectPath(RenderItem(id, "layer", 1.0, "normal", Rect(0.0, 0.0, 200.0, 200.0),
            Contours(longArrayOf(), longArrayOf(), uintArrayOf()), Transform(), style, shape, false))
        val objects = listOf(objectFor("ellipse", Shape.Ellipse(Rect(20.0, 20.0, 100.0, 100.0))),
            objectFor("line", Shape.Line(listOf(Point(150.0, 10.0), Point(150.0, 150.0)))))
        val doc = DocumentSnapshot("document", "Fixture", 200u, 200u, 8u, listOf(LayerInfo("layer", "Annotations", true, false, 1.0, "normal")), RenderList(info, objects.map { it.item }))
        val scene = CanvasScene(1, doc, null, Camera(Point(100.0, 100.0), 1.0, 0.0, 200.0, 200.0), Transform(), objects)
        assertTrue(EditorDrawing.eraseHits(scene, Point(70.0, 70.0), Point(70.0, 70.0), 3f).isEmpty())
        assertEquals(setOf("line"), EditorDrawing.eraseHits(scene, Point(130.0, 80.0), Point(170.0, 80.0), 2f))
        assertNull(EditorDrawing.hit(scene, Point(70.0, 70.0), 2f))
    }

    @Test fun bundledTextOutlinesHaveOneToOneAndroidAndExportPixelComparison() = runBlocking {
        val fixture = Fixture(); var project: WorkbenchProject? = null
        val bitmaps = mutableListOf<Bitmap>()
        try {
            project = fixture.create()
            val labels = listOf("Inter" to "ffi Á Bézier", "Noto Sans" to "abc \u202e123\u202c Ω Ж", "Noto Sans Mono" to "0123456789 {}")
            project.edit(fixture.options(labels.size), labels.mapIndexed { i, (font, label) -> EditCommand.Create(fixture.id(), fixture.layerId,
                Shape.Text(Point(12.0, 18.0 + i * 64), label, font, 30.0), ObjectStyle(0x000000ffu, 1.0)) })
            val doc = project.document(fixture.documentId)
            assertTrue(doc.render.items.all { (it.shape as Shape.Text).outline.isNotEmpty() })
            val camera = Camera(Point(240.0, 120.0), 1.0, 0.0, 480.0, 240.0)
            val android = Bitmap.createBitmap(480, 240, Bitmap.Config.ARGB_8888).also(bitmaps::add)
            val scene = CanvasScene(1, doc, null, camera, fixture.core.cameraMatrix(camera), doc.render.items.map(EditorDrawing::objectPath))
            EditorDrawing.draw(Canvas(android), scene)
            val encoded = project.export(ExportOptions(fixture.documentId, assumeUntaggedSrgb = true))
            val exported = requireNotNull(BitmapFactory.decodeByteArray(encoded.bytes, 0, encoded.bytes.size)).also(bitmaps::add)
            val a = IntArray(480 * 240); val b = IntArray(a.size)
            android.getPixels(a, 0, 480, 0, 0, 480, 240); exported.getPixels(b, 0, 480, 0, 0, 480, 240)
            var changed = 0; var solidMismatch = 0; var absoluteError = 0L; var maxError = 0
            fun edge(pixels: IntArray, index: Int): Boolean {
                val value = pixels[index] and 255
                if (value in 1..254) return true
                val x = index % 480; val y = index / 480
                for (dy in -1..1) for (dx in -1..1) if (x + dx in 0 until 480 && y + dy in 0 until 240 && (pixels[(y + dy) * 480 + x + dx] and 255) != value) return true
                return false
            }
            for (i in a.indices) {
                val difference = abs((a[i] and 255) - (b[i] and 255)); absoluteError += difference; maxError = maxOf(maxError, difference)
                if (a[i] != b[i]) { changed++; if (!edge(a, i) && !edge(b, i)) solidMismatch++ }
            }
            val mae = absoluteError.toDouble() / a.size
            Log.i("VW_TEXT_GOLDEN", "source=bundled_core_outlines scale=1 changed_pixels=$changed total_pixels=${a.size} solid_mismatch=$solidMismatch mae_8bit=$mae max_error_8bit=$maxError exact_match=${changed == 0} images_written=0")
            assertEquals("Text interiors shifted or changed; antialiasing cannot explain this", 0, solidMismatch)
            assertTrue("Text differs beyond bounded edge rasterization: mean error $mae", mae <= 2.0)
            // The receipt reports exact_match independently. This test's edge
            // tolerance must never be reported as bit-exact T1.08b acceptance.
        } finally { bitmaps.forEach(Bitmap::recycle); project?.close(); fixture.clean() }
    }

    @Test fun bothThemesMeetAaForAllDeclaredTextPairs() {
        for ((name, colors) in listOf("dark" to WorkbenchPalette.dark, "light" to WorkbenchPalette.light)) {
            val pairs = listOf("body" to (colors.onSurface to colors.surface), "background" to (colors.onBackground to colors.background),
                "accent" to (colors.primary to colors.surface), "muted" to (colors.secondary to colors.surface),
                "button" to (colors.onPrimary to colors.primary), "error" to (colors.onError to colors.error))
            for ((label, pair) in pairs) {
                val ratio = WorkbenchPalette.contrast(pair.first, pair.second)
                Log.i("VW_CONTRAST", "theme=$name pair=$label ratio=$ratio minimum=4.5")
                assertTrue("$name $label contrast $ratio", ratio >= 4.5)
            }
        }
    }
}
