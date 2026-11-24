package com.visualworkbench.android

import android.app.Instrumentation
import android.graphics.Bitmap
import android.graphics.Color
import android.os.Bundle
import android.os.SystemClock
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.PictureSurface
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.util.ArrayDeque
import java.util.concurrent.atomic.AtomicReference

/** Every picture a [PictureSurface] hands back is the one just drawn, never
 * an earlier one the platform renderer drew late. Pictures are drawn one after
 * another as fast as the renderer takes them, each a solid colour of its own,
 * so that the renderer takes many of them for a second frame within one display
 * frame and skips them, which sends the render to a new target. Each picture
 * is read back and must show its own colour; pictures held while later ones
 * are drawn must still show theirs when they are given back. Off screen, on
 * the renderer that draws the canvas; nothing here involves the app's UI. */
@RunWith(AndroidJUnit4::class)
class PictureSurfaceInstrumentedTest {
    private val instrumentation: Instrumentation get() = InstrumentationRegistry.getInstrumentation()

    /** A colour no other render of the run has: the index spread over the RGB cube. */
    private fun colour(index: Int): Int {
        val n = index + 1
        return Color.rgb((n * 37) and 0xff, (n * 101 + 17) and 0xff, (n * 59 + 113) and 0xff)
    }

    private class Drawn(val picture: PictureSurface.Picture, val colour: Int, val index: Int)

    /** The colours of [picture] at its centre and its four corners. */
    private fun shows(picture: PictureSurface.Picture, colour: Int): String? {
        val pixels = picture.bitmap.copy(Bitmap.Config.ARGB_8888, false) ?: return "it could not be read back"
        try {
            val w = pixels.width; val h = pixels.height
            for ((x, y) in listOf(w / 2 to h / 2, 0 to 0, w - 1 to 0, 0 to h - 1, w - 1 to h - 1)) {
                val found = pixels.getPixel(x, y)
                if (found != colour) return String.format("%08x at %d,%d instead of %08x", found, x, y, colour)
            }
            return null
        } finally { pixels.recycle() }
    }

    /** Runs [body] on a thread of its own, as the canvas's drawing thread is. */
    private fun onThread(body: () -> Unit) {
        val failure = AtomicReference<Throwable?>(null)
        val thread = Thread({ try { body() } catch (error: Throwable) { failure.set(error) } }, "picture-test")
        thread.start(); thread.join(120_000)
        assertTrue("The renders did not finish within two minutes", !thread.isAlive)
        failure.get()?.let { throw it }
    }

    @Test fun everyPictureShowsWhatWasDrawnForItAlsoWhenTheRendererSkipsAFrame() {
        val notes = StringBuilder()
        val duplicatesBefore = PictureSurface.duplicates.get()
        onThread {
            val surface = PictureSurface(1080, 1216)
            try {
                var index = 0
                var retriedBefore = 0
                fun render(): Drawn {
                    val at = index++
                    val made = surface.render { it.drawColor(colour(at)) }
                    assertNotNull("Render $at came out as nothing (retried ${surface.retried} so far)", made)
                    return Drawn(requireNotNull(made), colour(at), at)
                }
                fun check(drawn: Drawn, phase: String) {
                    shows(drawn.picture, drawn.colour)?.let { throw AssertionError("$phase: picture ${drawn.index} shows $it") }
                }

                // Two at a time, the second at once after the first: the renderer
                // takes the second for the same display frame.
                repeat(40) {
                    val first = render(); val second = render()
                    check(first, "pairs"); check(second, "pairs")
                    first.picture.release(); second.picture.release()
                }
                notes.append("pairs: 80 renders, ${surface.retried - retriedBefore} made again on a new target\n")
                retriedBefore = surface.retried

                // Three held while more are drawn, two at once each time: the reader
                // is as full as it may be when the newest image is taken.
                val held = ArrayDeque<Drawn>()
                repeat(30) {
                    while (held.size < 2) held.addLast(render().also { check(it, "filling") })
                    val third = render(); val fourth = render()
                    check(third, "held"); check(fourth, "held")
                    held.addLast(third); held.addLast(fourth)
                    // The oldest two are given back; they must not have been drawn over.
                    repeat(2) { val oldest = held.removeFirst(); check(oldest, "given back"); oldest.picture.release() }
                }
                while (held.isNotEmpty()) held.removeFirst().let { check(it, "given back"); it.picture.release() }
                notes.append("held: ${index - 80} renders with up to three pictures held, ${surface.retried - retriedBefore} made again\n")
                retriedBefore = surface.retried

                // At an ordinary pace: one at a time, a display frame or more apart.
                repeat(20) {
                    SystemClock.sleep(20)
                    val one = render(); check(one, "paced"); one.picture.release()
                }
                notes.append("paced: 20 renders, ${surface.retried - retriedBefore} made again\n")
                notes.append("total: $index renders, ${surface.retried} made again on a new target, " +
                    "${PictureSurface.duplicates.get() - duplicatesBefore} older images found behind the newest\n")
                // The skip this guards against must actually have happened.
                assertTrue("No render was made again on a new target, so the skipped-frame path was not run.\n$notes", surface.retried > 0)
            } finally { surface.close() }
        }
        Log.i(TAG, notes.toString())
        instrumentation.sendStatus(0, Bundle().apply { putString(Instrumentation.REPORT_KEY_STREAMRESULT, "\n$notes") })
        assertEquals("Targets left open", 0, PictureSurface.liveSurfaces.get())
        assertEquals("Pictures not given back", 0, PictureSurface.livePictures.get())
    }

    private companion object { const val TAG = "VW_PICTURE_SURFACE" }
}
