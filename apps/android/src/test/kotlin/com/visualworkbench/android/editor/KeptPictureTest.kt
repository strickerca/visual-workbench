@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

/** The rules of the kept picture: when a frame may be drawn from it, when it
 * stands as it is, when a finished stroke is drawn into it and when it has to
 * be made again; and which saved objects a magnifier pane may leave out. The
 * drawing itself is framework code and is judged on the device
 * (KeptPictureInstrumentedTest). */
class KeptPictureTest {
    private fun item(id: String, layer: String, opacity: Double = 1.0, blend: String = "normal", shape: Shape = Shape.Stroke("pen")) =
        RenderItem(id, layer, opacity, blend, Rect(0.0, 0.0, 1.0, 1.0), Contours(longArrayOf(), longArrayOf(), uintArrayOf()),
            Transform(), ObjectStyle(0x203040ffu, 4.0), shape, false)
    private val layers = listOf("notes", "top")

    // region What may be drawn on top of a picture
    @Test fun ink_in_the_layer_drawn_last_with_plain_compositing_may_go_on_top() {
        // Empty document, one layer with strokes, and the top one of two layers.
        assertTrue(wetInkOnTop(listOf("notes"), emptyList(), listOf(item("w", "notes")), emptySet()))
        assertTrue(wetInkOnTop(listOf("notes"), listOf(item("a", "notes"), item("b", "notes")), listOf(item("w", "notes")), emptySet()))
        assertTrue(wetInkOnTop(layers, listOf(item("a", "notes"), item("b", "top")), listOf(item("w", "top")), emptySet()))
        // A layer above that has nothing drawn does not cover the ink.
        assertTrue(wetInkOnTop(layers, listOf(item("a", "notes")), listOf(item("w", "notes")), emptySet()))
        // Hidden objects above are not drawn either.
        assertTrue(wetInkOnTop(layers, listOf(item("a", "notes"), item("b", "top")), listOf(item("w", "notes")), setOf("b")))
        // A layer the document does not list yet is drawn after every other one.
        assertTrue(wetInkOnTop(layers, listOf(item("a", "notes"), item("b", "top")), listOf(item("w", "fresh")), emptySet()))
        // Two strokes in flight in the same top layer.
        assertTrue(wetInkOnTop(listOf("notes"), listOf(item("a", "notes")), listOf(item("w1", "notes"), item("w2", "notes")), emptySet()))
    }
    @Test fun ink_under_another_layer_needs_the_whole_scene() {
        assertFalse(wetInkOnTop(layers, listOf(item("a", "notes"), item("b", "top")), listOf(item("w", "notes")), emptySet()))
        // Also when the covering layer is one the document does not list.
        assertFalse(wetInkOnTop(listOf("notes"), listOf(item("a", "notes"), item("b", "later")), listOf(item("w", "notes")), emptySet()))
        // Ink spread over two layers is never taken apart.
        assertFalse(wetInkOnTop(layers, emptyList(), listOf(item("w1", "notes"), item("w2", "top")), emptySet()))
    }
    @Test fun a_multiply_or_translucent_layer_needs_the_whole_scene() {
        assertFalse(wetInkOnTop(listOf("notes"), emptyList(), listOf(item("w", "notes", blend = "multiply")), emptySet()))
        assertFalse(wetInkOnTop(listOf("notes"), emptyList(), listOf(item("w", "notes", opacity = 0.99)), emptySet()))
        // The layer is composited the way its first drawn object says, not the way the wet stroke says.
        assertFalse(wetInkOnTop(listOf("notes"), listOf(item("a", "notes", blend = "multiply")), listOf(item("w", "notes")), emptySet()))
        assertFalse(wetInkOnTop(listOf("notes"), listOf(item("a", "notes", opacity = 0.5)), listOf(item("w", "notes")), emptySet()))
        assertTrue(wetInkOnTop(listOf("notes"), listOf(item("a", "notes")), listOf(item("w", "notes", blend = "multiply")), emptySet()))
        // A hidden first object does not decide; the first drawn one does.
        assertTrue(wetInkOnTop(listOf("notes"), listOf(item("a", "notes", opacity = 0.5), item("b", "notes")), listOf(item("w", "notes")), setOf("a")))
        // Result pixels are drawn by their own route.
        assertFalse(wetInkOnTop(listOf("notes"), emptyList(), listOf(item("w", "notes", shape = Shape.Result("asset", "result"))), emptySet()))
        assertFalse(wetInkOnTop(listOf("notes"), listOf(item("a", "notes", shape = Shape.Result("asset", "result"))), listOf(item("w", "notes")), emptySet()))
    }
    @Test fun no_wet_ink_means_there_is_nothing_to_draw_on_top() {
        assertFalse(wetInkOnTop(listOf("notes"), listOf(item("a", "notes")), emptyList(), emptySet()))
        assertFalse(wetInkOnTop(listOf("notes"), listOf(item("a", "notes")), listOf(item("w", "notes")), setOf("w")))
    }
    @Test fun the_saved_part_is_worked_out_once_and_asked_for_every_frame() {
        val saved = DryLayers(layers, listOf(item("a", "notes"), item("b", "top", opacity = 0.5)), emptySet())
        assertFalse(saved.inkOnTop(listOf(item("w", "top")), emptySet()))
        assertFalse(saved.inkOnTop(listOf(item("w", "notes")), emptySet()))
        assertTrue(saved.inkOnTop(listOf(item("w", "fresh")), emptySet()))
    }
    /** After a stroke is saved the layer order is extended by that stroke, not
     * worked out again from every object: the answers must be the same. */
    @Test fun objects_added_at_the_end_give_the_same_answers_as_starting_over() {
        val hidden = setOf("h")
        val saved = listOf(item("a", "notes"), item("h", "top"), item("b", "notes"), item("c", "fresh", opacity = 0.5),
            item("d", "top"), item("e", "later", blend = "multiply"), item("f", "notes"))
        val probes = listOf("notes", "top", "fresh", "later", "unseen").flatMap { layer ->
            listOf(listOf(item("w", layer)), listOf(item("w", layer, blend = "multiply")), listOf(item("w", layer), item("x", "notes")))
        }
        for (split in 0..saved.size) {
            val grown = DryLayers(layers, saved.take(split), hidden)
            val held = grown.copy()
            grown.add(saved.drop(split), hidden)
            val whole = DryLayers(layers, saved, hidden)
            val before = DryLayers(layers, saved.take(split), hidden)
            for (probe in probes) {
                assertEquals("split=$split probe=${probe.map { it.layerId }}", whole.inkOnTop(probe, hidden), grown.inkOnTop(probe, hidden))
                // A copy taken before the objects were added still answers for what it was taken from.
                assertEquals("split=$split probe=${probe.map { it.layerId }}", before.inkOnTop(probe, hidden), held.inkOnTop(probe, hidden))
            }
        }
    }
    // endregion

    // region Which frames a picture can draw
    private fun refusal(hardware: Boolean = true, canvasWidth: Int = 1080, canvasHeight: Int = 2000, width: Int = 1080, height: Int = 2000,
        resizing: Boolean = false, giant: Boolean = false, documentTiles: Boolean = false, resultsReady: Boolean = true, results: Boolean = false,
        draft: Boolean = false, preview: Boolean = false, erasing: Boolean = false) =
        pictureRefusal(hardware, canvasWidth, canvasHeight, width, height, resizing, giant, documentTiles, resultsReady, results, draft, preview, erasing)

    @Test fun an_ordinary_scene_is_drawn_from_the_picture() { assertNull(refusal()) }

    @Test fun scenes_a_picture_is_never_kept_for_give_it_back() {
        val cases = listOf(
            refusal(hardware = false) to PictureRefusal.Software,
            refusal(giant = true) to PictureRefusal.Giant,
            refusal(documentTiles = true) to PictureRefusal.DocumentTiles,
            refusal(results = true) to PictureRefusal.Results,
            refusal(resultsReady = false) to PictureRefusal.Results,
            refusal(resizing = true) to PictureRefusal.Resizing)
        for ((actual, expected) in cases) { assertEquals(expected, actual); assertFalse(expected.name, expected.keepsPicture) }
    }
    @Test fun scenes_that_last_for_a_gesture_leave_the_picture_for_the_frames_after() {
        val cases = listOf(
            refusal(draft = true) to PictureRefusal.Draft,
            refusal(preview = true) to PictureRefusal.Preview,
            refusal(erasing = true) to PictureRefusal.Erasing,
            // A frame that is not the size the scene was laid out for, or a scene with no size yet.
            refusal(canvasWidth = 2000, canvasHeight = 1080) to PictureRefusal.Size,
            refusal(canvasHeight = 1999) to PictureRefusal.Size,
            refusal(width = 0, canvasWidth = 0) to PictureRefusal.Size,
            refusal(height = -1, canvasHeight = -1) to PictureRefusal.Size)
        for ((actual, expected) in cases) { assertEquals(expected, actual); assertTrue(expected.name, expected.keepsPicture) }
    }
    @Test fun a_reason_to_give_the_picture_back_comes_before_one_to_keep_it() {
        assertEquals(PictureRefusal.Giant, refusal(giant = true, erasing = true, preview = true, draft = true))
        assertEquals(PictureRefusal.Results, refusal(results = true, canvasWidth = 5))
        assertEquals(PictureRefusal.Resizing, refusal(resizing = true, canvasWidth = 5, erasing = true))
        assertEquals(PictureRefusal.Software, refusal(hardware = false, giant = true, resizing = true))
        // Every reason is one of the two kinds, and both kinds exist.
        assertEquals(setOf(true, false), PictureRefusal.entries.map { it.keepsPicture }.toSet())
    }
    // endregion

    // region Keep, add a stroke, or make again
    private val view = Transform(a = 1.5, d = 1.5, e = 20.0, f = 40.0)
    private val page = Any()
    private fun basis(objects: List<RenderItem>, width: Int = 1080, height: Int = 2000, matrix: Transform = view, scale: Double = 1.5,
        background: Any? = page, smooth: Boolean = true, layerIds: List<String> = layers, hidden: Set<String> = emptySet()) =
        PictureBasis(width, height, matrix, scale, background, smooth, layerIds, hidden, objects)
    private fun step(held: PictureBasis<RenderItem>?, next: PictureBasis<RenderItem>): PictureStep =
        pictureStep(held, held?.let { DryLayers(it.layerIds, it.objects, it.hidden) }, next) { it }
    private val a = item("a", "notes"); private val b = item("b", "top"); private val c = item("c", "top")
    private val saved = listOf(a, b)

    @Test fun with_no_picture_it_is_made() {
        assertEquals(PictureStep.Rebuild(PictureChange.Missing), step(null, basis(saved)))
        assertEquals(PictureStep.Rebuild(PictureChange.Missing), pictureStep(basis(saved), null, basis(saved)) { it })
    }
    @Test fun the_same_saved_scene_keeps_the_picture() {
        val held = basis(saved)
        assertEquals(PictureStep.Keep, step(held, held))
        assertEquals(PictureStep.Keep, step(held, basis(saved)))
        // The same objects in another list, and an equal set of hidden ids in another set.
        assertEquals(PictureStep.Keep, step(held, basis(saved.toList())))
        assertEquals(PictureStep.Keep, step(basis(saved, hidden = setOf("b")), basis(saved, hidden = HashSet(listOf("b")))))
        assertEquals(PictureStep.Keep, step(basis(emptyList()), basis(emptyList())))
    }
    @Test fun one_stroke_added_on_top_is_drawn_into_the_picture_and_nothing_else_is() {
        assertEquals(PictureStep.Append(2), step(basis(saved), basis(saved + c)))
        // The first stroke on an empty page, and the first in a layer above everything.
        assertEquals(PictureStep.Append(0), step(basis(emptyList()), basis(listOf(a))))
        assertEquals(PictureStep.Append(2), step(basis(saved), basis(saved + item("n", "fresh"))))
        // Strokes saved faster than frames are drawn arrive together.
        assertEquals(PictureStep.Append(2), step(basis(saved), basis(saved + c + item("d", "top") + item("e", "top"))))
        // What is hidden above does not count as drawn above.
        assertEquals(PictureStep.Append(2), step(basis(saved, hidden = setOf("b")), basis(saved + item("n", "notes"), hidden = setOf("b"))))
    }
    @Test fun a_stroke_that_is_not_plainly_on_top_makes_the_picture_again() {
        val again = PictureStep.Rebuild(PictureChange.Objects)
        // Under another layer; in a multiply (highlighter) layer; in a translucent layer; an AI result.
        assertEquals(again, step(basis(saved), basis(saved + item("n", "notes"))))
        assertEquals(again, step(basis(saved), basis(saved + item("n", "fresh", blend = "multiply"))))
        val faint = listOf(a, item("t", "top", opacity = 0.5))
        assertEquals(again, step(basis(faint), basis(faint + item("n", "top"))))
        assertEquals(again, step(basis(saved), basis(saved + item("n", "fresh", shape = Shape.Result("asset", "result")))))
        // Two added strokes in two layers.
        assertEquals(again, step(basis(saved), basis(saved + c + item("n", "fresh"))))
        // Added, but hidden: there is nothing to draw on top, and the picture is made from what is shown.
        val first = basis(saved, hidden = setOf("n"))
        assertEquals(again, step(first, basis(saved + item("n", "top"), hidden = setOf("n"))))
    }
    @Test fun any_other_change_to_the_saved_objects_makes_the_picture_again() {
        val again = PictureStep.Rebuild(PictureChange.Objects)
        val held = basis(listOf(a, b, c))
        // Undo or an erasure takes an object away; redo puts another object in its place.
        assertEquals(again, step(held, basis(listOf(a, b))))
        assertEquals(again, step(held, basis(listOf(a, b, item("c", "top")))))
        // An edit, a move or a change from the paired computer arrives as another object.
        assertEquals(again, step(held, basis(listOf(a, b.copy(transform = Transform(e = 5.0)), c))))
        assertEquals(again, step(held, basis(listOf(a.copy(style = ObjectStyle(0xff0000ffu, 4.0)), b, c))))
        // A layer's opacity or blend is carried by its objects.
        assertEquals(again, step(held, basis(listOf(a, b.copy(layerOpacity = 0.5), c.copy(layerOpacity = 0.5)))))
        // The same objects in another order, an object added anywhere but at the end, another document or page.
        assertEquals(again, step(held, basis(listOf(a, c, b))))
        assertEquals(again, step(held, basis(listOf(item("n", "top"), a, b, c))))
        assertEquals(again, step(held, basis(listOf(a, item("n", "top"), b, c))))
        assertEquals(again, step(held, basis(listOf(item("x", "notes"), item("y", "top"), item("z", "top")))))
        assertEquals(again, step(held, basis(emptyList())))
    }
    @Test fun a_change_of_view_makes_the_picture_again() {
        val held = basis(saved)
        // Pan, zoom and rotation all change how the document lies on the screen.
        assertEquals(PictureStep.Rebuild(PictureChange.View), step(held, basis(saved, matrix = view.copy(e = 21.0))))
        assertEquals(PictureStep.Rebuild(PictureChange.View), step(held, basis(saved, matrix = Transform(a = 3.0, d = 3.0, e = 20.0, f = 40.0), scale = 3.0)))
        assertEquals(PictureStep.Rebuild(PictureChange.View), step(held, basis(saved, matrix = Transform(a = 0.0, b = 1.5, c = -1.5, d = 0.0, e = 20.0, f = 40.0))))
        assertEquals(PictureStep.Rebuild(PictureChange.View), step(held, basis(saved, scale = 1.25)))
        // Another size of the canvas, as after turning the device or showing a tool's controls.
        assertEquals(PictureStep.Rebuild(PictureChange.Size), step(held, basis(saved, width = 2000, height = 1080)))
        assertEquals(PictureStep.Rebuild(PictureChange.Size), step(held, basis(saved, height = 1800)))
    }
    @Test fun a_change_of_background_layers_or_hidden_objects_makes_the_picture_again() {
        val held = basis(saved)
        // Another page picture, none at all, or the same one shown smoothed or not.
        assertEquals(PictureStep.Rebuild(PictureChange.Background), step(held, basis(saved, background = Any())))
        assertEquals(PictureStep.Rebuild(PictureChange.Background), step(held, basis(saved, background = null)))
        assertEquals(PictureStep.Rebuild(PictureChange.Background), step(held, basis(saved, smooth = false)))
        // Layers put in another order, added or removed.
        assertEquals(PictureStep.Rebuild(PictureChange.Layers), step(held, basis(saved, layerIds = listOf("top", "notes"))))
        assertEquals(PictureStep.Rebuild(PictureChange.Layers), step(held, basis(saved, layerIds = layers + "fresh")))
        // An object hidden or shown again.
        assertEquals(PictureStep.Rebuild(PictureChange.Hidden), step(held, basis(saved, hidden = setOf("a"))))
        assertEquals(PictureStep.Rebuild(PictureChange.Hidden), step(basis(saved, hidden = setOf("a")), basis(saved)))
    }
    @Test fun a_stroke_added_together_with_another_change_makes_the_picture_again() {
        val held = basis(saved)
        assertEquals(PictureStep.Rebuild(PictureChange.View), step(held, basis(saved + c, matrix = view.copy(f = 0.0))))
        assertEquals(PictureStep.Rebuild(PictureChange.Background), step(held, basis(saved + c, background = Any())))
        assertEquals(PictureStep.Rebuild(PictureChange.Hidden), step(held, basis(saved + c, hidden = setOf("a"))))
        assertEquals(PictureStep.Rebuild(PictureChange.Size), step(held, basis(saved + c, width = 900)))
    }
    /** Each stroke is drawn into the picture once: after it has been added the
     * picture stands, however often the scene is shown again. */
    @Test fun a_stroke_already_added_is_not_added_again() {
        val first = basis(saved); val second = basis(saved + c); val third = basis(saved + c + item("d", "top"))
        assertEquals(PictureStep.Append(2), step(first, second))
        assertEquals(PictureStep.Keep, step(second, second))
        assertEquals(PictureStep.Append(3), step(second, third))
        assertEquals(PictureStep.Keep, step(third, basis(third.objects.toList())))
    }
    /** While the page is panned or zoomed every frame wants another picture.
     * Making one for each would cost more than drawing the frame whole. */
    @Test fun a_picture_is_not_made_for_every_frame_of_a_movement() {
        // A change after a quiet time, and the first there ever was: made at once.
        assertTrue(pictureMadeNow(sameAsLastFrame = false, sinceChangeMs = -1))
        assertTrue(pictureMadeNow(sameAsLastFrame = false, sinceChangeMs = PICTURE_STREAK_MS))
        assertTrue(pictureMadeNow(sameAsLastFrame = false, sinceChangeMs = 5_000))
        // Another change right after the last: this frame is drawn whole.
        assertFalse(pictureMadeNow(sameAsLastFrame = false, sinceChangeMs = 0))
        assertFalse(pictureMadeNow(sameAsLastFrame = false, sinceChangeMs = 8))
        assertFalse(pictureMadeNow(sameAsLastFrame = false, sinceChangeMs = PICTURE_STREAK_MS - 1))
        // The movement has stopped: the next frame wants what the last one wanted, however soon it comes.
        assertTrue(pictureMadeNow(sameAsLastFrame = true, sinceChangeMs = 0))
        assertTrue(pictureMadeNow(sameAsLastFrame = true, sinceChangeMs = 8))
    }
    // endregion

    @Test fun a_pane_leaves_out_only_objects_that_cannot_reach_it() {
        val visible = Rect(100.0, 100.0, 50.0, 40.0)
        assertTrue(boundsMayTouch(Rect(120.0, 110.0, 5.0, 5.0), visible, 0.0))
        assertTrue(boundsMayTouch(Rect(0.0, 0.0, 1000.0, 1000.0), visible, 0.0))
        // Touching edges and anything within the margin count as reaching it.
        assertTrue(boundsMayTouch(Rect(150.0, 100.0, 5.0, 5.0), visible, 0.0))
        assertTrue(boundsMayTouch(Rect(152.0, 100.0, 5.0, 5.0), visible, 2.0))
        assertFalse(boundsMayTouch(Rect(152.5, 100.0, 5.0, 5.0), visible, 2.0))
        assertFalse(boundsMayTouch(Rect(90.0, 100.0, 7.0, 5.0), visible, 2.0))
        assertFalse(boundsMayTouch(Rect(120.0, 90.0, 5.0, 7.0), visible, 2.0))
        assertFalse(boundsMayTouch(Rect(120.0, 143.0, 5.0, 5.0), visible, 2.0))
        // A zero-size box (a dot) inside is kept; a box that cannot be judged is always kept.
        assertTrue(boundsMayTouch(Rect(125.0, 120.0, 0.0, 0.0), visible, 0.0))
        assertTrue(boundsMayTouch(Rect(Double.NaN, 0.0, 1.0, 1.0), visible, 2.0))
        assertTrue(boundsMayTouch(Rect(0.0, 0.0, Double.POSITIVE_INFINITY, 1.0), visible, 2.0))
        assertTrue(boundsMayTouch(Rect(0.0, 0.0, 1.0, 1.0), visible, Double.NaN))
    }
    @Test fun the_document_area_under_a_pane_follows_scale_shift_and_rotation() {
        // 4x magnification, document point (10, 20) at pane corner (300, 500).
        val matrix = Transform(a = 4.0, d = 4.0, e = 300.0 - 40.0, f = 500.0 - 80.0)
        val area = requireNotNull(visibleDocumentRect(matrix, Rect(300.0, 500.0, 200.0, 100.0)))
        assertEquals(10.0, area.x, 1e-9); assertEquals(20.0, area.y, 1e-9)
        assertEquals(50.0, area.width, 1e-9); assertEquals(25.0, area.height, 1e-9)
        // A quarter turn: the box of the turned area still holds every corner.
        val turned = Transform(a = 0.0, b = 2.0, c = -2.0, d = 0.0, e = 100.0, f = 0.0)
        val box = requireNotNull(visibleDocumentRect(turned, Rect(0.0, 0.0, 100.0, 60.0)))
        for ((x, y) in listOf(0.0 to 0.0, 100.0 to 0.0, 0.0 to 60.0, 100.0 to 60.0)) {
            // Inverse of (x, y) = (-2v + 100, 2u): u = y / 2, v = (100 - x) / 2.
            val u = y / 2; val v = (100.0 - x) / 2
            assertTrue(u >= box.x - 1e-9 && u <= box.x + box.width + 1e-9 && v >= box.y - 1e-9 && v <= box.y + box.height + 1e-9)
        }
        assertEquals(30.0, box.width, 1e-9); assertEquals(50.0, box.height, 1e-9)
        assertNull(visibleDocumentRect(Transform(a = 0.0, d = 0.0), Rect(0.0, 0.0, 10.0, 10.0)))
        assertNull(visibleDocumentRect(Transform(a = Double.NaN), Rect(0.0, 0.0, 10.0, 10.0)))
    }
}
