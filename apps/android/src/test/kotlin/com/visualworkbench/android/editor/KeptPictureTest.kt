@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

/** The rules of the kept pictures: which layers have one, when a frame may
 * be drawn from them, when a finished stroke is drawn into its layer's
 * picture, when a part of one is drawn again and when one is drawn whole; how
 * pictures lie in a frame while the view moves, and when they are drawn at the
 * view again; and which saved objects a part of the page may leave out. The
 * drawing itself is framework code and is judged on the device
 * (KeptPictureInstrumentedTest). */
class KeptPictureTest {
    private fun item(id: String, layer: String, opacity: Double = 1.0, blend: String = "normal", shape: Shape = Shape.Stroke("pen")) =
        RenderItem(id, layer, opacity, blend, Rect(0.0, 0.0, 1.0, 1.0), Contours(longArrayOf(), longArrayOf(), uintArrayOf()),
            Transform(), ObjectStyle(0x203040ffu, 4.0), shape, false)

    // region Layers: one picture each, composited for every frame
    private fun stroke(id: String, layer: String, opacity: Double = 1.0, blend: String = "normal", contours: Contours = Contours(longArrayOf(0, 256, 0), longArrayOf(0, 0, 256), uintArrayOf(3u))) =
        RenderItem(id, layer, opacity, blend, Rect(0.0, 0.0, 1.0, 1.0), contours, Transform(), ObjectStyle(0x203040ffu, 4.0), Shape.Stroke("pen"), false)

    @Test fun each_layer_s_objects_are_listed_in_drawing_order_and_the_layers_as_the_scene_draws_them() {
        val objects = listOf(item("a", "notes"), item("u", "unlisted"), item("b", "top"), item("c", "notes"), item("v", "later"), item("d", "top"))
        val lists = layerLists(listOf("top", "notes", "empty"), objects) { it }
        // The document's layers first, in its order; then the others by first use. An empty layer has no list.
        assertEquals(listOf("top", "notes", "unlisted", "later"), lists.order)
        assertEquals(listOf("b", "d"), lists.of("top").map { it.objectId })
        assertEquals(listOf("a", "c"), lists.of("notes").map { it.objectId })
        assertTrue(lists.of("empty").isEmpty())
        assertEquals(setOf("top", "notes", "unlisted", "later"), lists.layers)
        // How a layer is composited comes from its first drawn object.
        assertEquals("c", firstDrawn(lists.of("notes"), setOf("a")) { it }?.objectId)
        assertNull(firstDrawn(lists.of("notes"), setOf("a", "c")) { it })
    }

    @Test fun each_layer_s_list_follows_the_page_s_list_by_what_changed() {
        val ids = listOf("base", "faint", "high")
        val base = (0 until 6).map { item("b$it", "base") }; val faint = (0 until 3).map { item("f$it", "faint") }; val high = (0 until 2).map { item("h$it", "high") }
        val page = base + faint + high
        val lists = layerLists(ids, page) { it }
        assertEquals(listOf(LayerRun("base", 0, 6), LayerRun("faint", 6, 9), LayerRun("high", 9, 11)), lists.runs)
        fun same(a: LayerLists<RenderItem>, b: LayerLists<RenderItem>) {
            assertEquals(b.order, a.order); assertEquals(b.layers, a.layers); assertEquals(b.runs, a.runs)
            for (layer in b.layers) assertEquals(b.of(layer), a.of(layer))
        }
        // A stroke added to the lowest layer, one taken from the middle one, a layer hidden, a layer's objects read again.
        for (next in listOf(base + item("n", "base") + faint + high, base + faint.drop(1) + high, base + faint, base + faint + high.map { it.copy(layerOpacity = 0.5) },
                            base + faint + high + item("u", "unlisted"))) {
            val followed = requireNotNull(followLayerLists(lists, page, next, ids, 1_024) { it })
            same(followed, layerLists(ids, next) { it })
            // A list followed again is followed from what it now holds.
            val again = base + item("z", "base") + faint + high
            followLayerLists(followed, next, again, ids, 1_024) { it }?.let { same(it, layerLists(ids, again) { it }) }
        }
        // A layer's objects no longer together, or more changed than is followed: worked out again from every object.
        assertNull(followLayerLists(lists, page, base + faint + high + item("b9", "base"), ids, 1_024) { it })
        assertNull(followLayerLists(lists, page, page.map { it.copy(layerOpacity = 0.9) }, ids, 4) { it })
        assertNull(layerLists(ids, listOf(item("a", "base"), item("b", "high"), item("c", "base"))) { it }.runs)
        // The box around the objects only grows.
        val grown = requireNotNull(followLayerLists(lists, page, page + item("far", "high").copy(bounds = Rect(500.0, 500.0, 10.0, 10.0)), ids, 1_024) { it })
        assertEquals(Rect(0.0, 0.0, 510.0, 510.0), grown.bounds)
    }

    @Test fun every_layer_that_draws_has_its_own_picture_up_to_four_then_the_lowest_share_one_with_the_source() {
        assertEquals(emptyList<SlotKey>(), slotLayout(emptyList(), 4))
        assertEquals(listOf(SlotKey(listOf("a"), false)), slotLayout(listOf("a"), 4))
        assertEquals(listOf("a", "b", "c", "d").map { SlotKey(listOf(it), false) }, slotLayout(listOf("a", "b", "c", "d"), 4))
        // Six layers: the topmost three alone, the three beneath with the source pixels.
        assertEquals(listOf(SlotKey(listOf("a", "b", "c"), true), SlotKey(listOf("d"), false), SlotKey(listOf("e"), false), SlotKey(listOf("f"), false)),
            slotLayout(listOf("a", "b", "c", "d", "e", "f"), 4))
        // However many layers draw, never more pictures than allowed.
        for (count in 0..20) assertTrue(slotLayout((0 until count).map { "l$it" }, 4).size <= 4)
    }

    @Test fun an_object_read_again_with_its_layer_composited_otherwise_draws_the_same() {
        val shape = item("r", "notes", shape = Shape.Rectangle(Rect(1.0, 2.0, 3.0, 4.0)))
        // Another opacity, blend or lock: drawn alike in the layer's own picture.
        assertTrue(sameDrawing(shape, shape.copy(layerOpacity = 0.4)))
        assertTrue(sameDrawing(shape, shape.copy(layerBlend = "multiply", locked = true)))
        assertTrue(sameDrawing(shape, shape.copy(contours = Contours(longArrayOf(), longArrayOf(), uintArrayOf()))))
        // Anything that changes what is drawn does not.
        assertFalse(sameDrawing(shape, shape.copy(objectId = "other")))
        assertFalse(sameDrawing(shape, shape.copy(layerId = "top")))
        assertFalse(sameDrawing(shape, shape.copy(transform = Transform(e = 1.0))))
        assertFalse(sameDrawing(shape, shape.copy(style = ObjectStyle(0xff0000ffu, 4.0))))
        assertFalse(sameDrawing(shape, shape.copy(shape = Shape.Rectangle(Rect(1.0, 2.0, 3.0, 5.0)))))
        assertFalse(sameDrawing(shape, shape.copy(bounds = Rect(0.0, 0.0, 9.0, 9.0))))
        // A stroke keeps its contours when its layer changes; other contours are another stroke.
        val pen = stroke("s", "notes")
        assertTrue(sameDrawing(pen, pen.copy(layerOpacity = 0.5)))
        assertFalse(sameDrawing(pen, stroke("s", "notes")))
    }

    @Test fun a_stroke_in_any_layer_is_added_to_that_layer_s_picture() {
        val a = item("a", "notes"); val b = item("b", "notes")
        // Beneath other layers, in a multiply (highlighter) layer, in a translucent one: each layer's list grows at its end.
        assertEquals(LayerStep.Append(2), layerStep(listOf(a, b), listOf(a, b, item("n", "notes")), 1_024) { it })
        val high = item("h", "high", blend = "multiply")
        assertEquals(LayerStep.Append(1), layerStep(listOf(high), listOf(high, item("n", "high", blend = "multiply")), 1_024) { it })
        val faint = item("t", "faint", opacity = 0.55)
        assertEquals(LayerStep.Append(1), layerStep(listOf(faint), listOf(faint, item("n", "faint", opacity = 0.55)), 1_024) { it })
        // The first stroke of a layer, and strokes saved faster than frames are drawn.
        assertEquals(LayerStep.Append(0), layerStep(emptyList(), listOf(a), 1_024) { it })
        assertEquals(LayerStep.Append(1), layerStep(listOf(a), listOf(a, b, item("c", "notes")), 1_024) { it })
    }

    @Test fun a_layer_composited_otherwise_or_put_elsewhere_keeps_its_picture() {
        val objects = (0 until 3_000).map { stroke("s$it", "notes") }
        // Every object of the layer read again with another opacity: more than are followed one by one, yet the same drawing.
        assertEquals(LayerStep.Same, layerStep(objects, objects.map { it.copy(layerOpacity = 0.3, layerBlend = "multiply") }, 1_024) { it })
        // Nothing changed, the same list or an equal one.
        assertEquals(LayerStep.Same, layerStep(objects, objects, 1_024) { it })
        assertEquals(LayerStep.Same, layerStep(objects, objects.toList(), 1_024) { it })
    }

    @Test fun other_changes_draw_a_part_of_the_layer_again_and_too_many_draw_it_whole() {
        val a = item("a", "notes"); val b = item("b", "notes"); val c = item("c", "notes")
        fun changed(step: LayerStep) = assertTrue("expected a change, got $step", step is LayerStep.Changed)
        // Undo, an erasure, an edit, an object put in beneath others, another order.
        changed(layerStep(listOf(a, b, c), listOf(a, b), 1_024) { it })
        changed(layerStep(listOf(a, b, c), listOf(a, b.copy(transform = Transform(e = 5.0)), c), 1_024) { it })
        changed(layerStep(listOf(a, b), listOf(item("n", "notes"), a, b), 1_024) { it })
        changed(layerStep(listOf(a, b, c), listOf(a, c, b), 1_024) { it })
        changed(layerStep(listOf(a), emptyList(), 1_024) { it })
        // More changed than is followed one by one.
        val many = (0 until 50).map { item("m$it", "notes") }
        assertEquals(LayerStep.Redraw, layerStep(many, many.map { it.copy(transform = Transform(e = 1.0)) }, 10) { it })
    }

    @Test fun what_each_picture_needs_follows_what_happened_to_its_layers() {
        val notes = SlotKey(listOf("notes"), false); val top = SlotKey(listOf("top"), false)
        val prefix = SlotKey(listOf("base", "notes"), true)
        val held = setOf(notes, top, prefix)
        val none = emptyMap<String, LayerStep>()
        assertEquals(SlotWork.Keep, slotWork(notes, held, none, false, emptySet()))
        // A layer shown, or one that drew nothing before: its picture is drawn.
        assertEquals(SlotWork.Render, slotWork(SlotKey(listOf("fresh"), false), held, none, false, emptySet()))
        assertEquals(SlotWork.Append, slotWork(notes, held, mapOf("notes" to LayerStep.Append(3)), false, emptySet()))
        assertEquals(SlotWork.Patch, slotWork(notes, held, mapOf("notes" to LayerStep.Changed(ObjectsDiff(intArrayOf(0), IntArray(0)))), false, emptySet()))
        assertEquals(SlotWork.Render, slotWork(notes, held, mapOf("notes" to LayerStep.Redraw), false, emptySet()))
        // Another layer's change, the source pixels and a layer composited otherwise leave a layer's own picture alone.
        assertEquals(SlotWork.Keep, slotWork(top, held, mapOf("notes" to LayerStep.Redraw), true, setOf("top")))
        // A picture that holds several layers and the source pixels draws strokes in by parts, and is drawn again
        // for another source, or for a layer of it composited otherwise.
        assertEquals(SlotWork.Patch, slotWork(prefix, held, mapOf("notes" to LayerStep.Append(1)), false, emptySet()))
        assertEquals(SlotWork.Render, slotWork(prefix, held, none, true, emptySet()))
        assertEquals(SlotWork.Render, slotWork(prefix, held, none, false, setOf("base")))
        assertEquals(SlotWork.Keep, slotWork(prefix, held, none, false, setOf("top")))
    }
    // endregion

    // region Which frames a picture can draw
    private fun refusal(hardware: Boolean = true, canvasWidth: Int = 1080, canvasHeight: Int = 2000, width: Int = 1080, height: Int = 2000,
        resizing: Boolean = false, giant: Boolean = false, documentTiles: Boolean = false, resultsReady: Boolean = true, results: Boolean = false,
        draft: Boolean = false) =
        pictureRefusal(hardware, canvasWidth, canvasHeight, width, height, resizing, giant, documentTiles, resultsReady, results, draft)

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
            // A frame that is not the size the scene was laid out for, or a scene with no size yet.
            refusal(canvasWidth = 2000, canvasHeight = 1080) to PictureRefusal.Size,
            refusal(canvasHeight = 1999) to PictureRefusal.Size,
            refusal(width = 0, canvasWidth = 0) to PictureRefusal.Size,
            refusal(height = -1, canvasHeight = -1) to PictureRefusal.Size)
        for ((actual, expected) in cases) { assertEquals(expected, actual); assertTrue(expected.name, expected.keepsPicture) }
        // A move, a turn or an erasure that cannot be drawn over the picture leaves it too.
        assertTrue(PictureRefusal.Gesture.keepsPicture)
    }
    @Test fun a_reason_to_give_the_picture_back_comes_before_one_to_keep_it() {
        assertEquals(PictureRefusal.Giant, refusal(giant = true, draft = true))
        assertEquals(PictureRefusal.Results, refusal(results = true, canvasWidth = 5))
        assertEquals(PictureRefusal.Resizing, refusal(resizing = true, canvasWidth = 5, draft = true))
        assertEquals(PictureRefusal.Software, refusal(hardware = false, giant = true, resizing = true))
        // Every reason is one of the two kinds, and both kinds exist.
        assertEquals(setOf(true, false), PictureRefusal.entries.map { it.keepsPicture }.toSet())
    }
    // endregion

    // region Views: pictures moved, scaled or turned, and drawn at the view once it rests
    private val view = Transform(a = 1.5, d = 1.5, e = 20.25, f = 40.75)

    @Test fun a_whole_pixel_pan_moves_the_pictures_and_is_exact_only_with_the_page_on_the_screen_whole() {
        assertEquals(ViewMove.Same, viewMove(view, view, true, true))
        assertEquals(ViewMove.Shift(37, -23, true), viewMove(view, view.copy(e = view.e + 37, f = view.f - 23), true, true))
        // The screen's edge cuts the page before or after: moved, but drawn at the view once it rests.
        assertEquals(ViewMove.Shift(37, -23, false), viewMove(view, view.copy(e = view.e + 37, f = view.f - 23), false, true))
        assertEquals(ViewMove.Shift(37, -23, false), viewMove(view, view.copy(e = view.e + 37, f = view.f - 23), true, false))
        // Arithmetic noise in a whole-pixel move still counts as whole; a part of a pixel does not.
        assertEquals(ViewMove.Shift(5, 0, true), viewMove(view, view.copy(e = view.e + 5 + 1e-9), true, true))
        assertEquals(ViewMove.Same, viewMove(view, view.copy(e = view.e + 1e-9), true, true))
        assertTrue(viewMove(view, view.copy(e = view.e + 0.5), true, true) is ViewMove.Warp)
        assertTrue(viewMove(view, view.copy(e = view.e + 5.01), true, true) is ViewMove.Warp)
    }

    @Test fun a_pinch_or_turn_draws_the_pictures_under_the_change_of_view() {
        // Zoomed in twice about the point (100, 200) of the screen.
        val zoomed = EditorDrawing.compose(Transform(a = 2.0, d = 2.0, e = -100.0, f = -200.0), view)
        val move = viewMove(view, zoomed, true, true) as ViewMove.Warp
        val expected = Transform(a = 2.0, d = 2.0, e = -100.0, f = -200.0)
        for ((got, want) in listOf(move.toScreen.a to expected.a, move.toScreen.d to expected.d, move.toScreen.e to expected.e, move.toScreen.f to expected.f))
            assertEquals(want, got, 1e-9)
        // A turn is a warp too, and a view that cannot be inverted cannot be judged.
        val turned = Transform(a = 0.0, b = 1.5, c = -1.5, d = 0.0, e = 20.0, f = 40.0)
        assertTrue(viewMove(view, turned, true, true) is ViewMove.Warp)
        assertNull(viewMove(Transform(a = 0.0, d = 0.0), view, true, true))
        // Which views have to be drawn again once they rest.
        assertTrue(softView(move)); assertTrue(softView(ViewMove.Shift(1, 1, false)))
        assertFalse(softView(ViewMove.Shift(1, 1, true))); assertFalse(softView(ViewMove.Same))
        // Where a warped picture lands: its four corners.
        val corners = pictureCorners(expected, 100, 50)
        assertArrayEquals(doubleArrayOf(-100.0, -200.0, 100.0, -200.0, 100.0, -100.0, -100.0, -100.0), corners, 1e-9)
    }

    @Test fun whether_the_page_lies_on_the_screen_whole() {
        // A 1000 x 2000 page at half size, 20 px in: inside a 1080 x 1216 screen.
        assertTrue(pageOnScreen(Transform(a = 0.5, d = 0.5, e = 20.0, f = 20.0), 1000.0, 2000.0, 1080, 1216))
        // Touching the edges counts as on it; a pixel beyond does not, nor a page bigger than the screen.
        assertTrue(pageOnScreen(Transform(a = 1.0, d = 1.0), 1080.0, 1216.0, 1080, 1216))
        assertFalse(pageOnScreen(Transform(a = 1.0, d = 1.0, e = 1.0), 1080.0, 1216.0, 1080, 1216))
        assertFalse(pageOnScreen(Transform(a = 3.0, d = 3.0), 1000.0, 2000.0, 1080, 1216))
        // Turned a quarter: the page's corners are what count.
        assertTrue(pageOnScreen(Transform(a = 0.0, b = 0.5, c = -0.5, d = 0.0, e = 1000.0, f = 10.0), 1000.0, 2000.0, 1080, 1216))
        assertFalse(pageOnScreen(Transform(a = Double.NaN), 10.0, 10.0, 1080, 1216))
    }

    @Test fun a_picture_moved_is_exact_only_when_nothing_in_it_is_cut_by_the_screen_s_edge() {
        val fitted = Transform(a = 0.5, d = 0.5, e = 20.0, f = 20.0)
        val within = layerLists(listOf("notes"), listOf(item("a", "notes").copy(bounds = Rect(10.0, 10.0, 500.0, 500.0)))) { it }
        assertEquals(Rect(10.0, 10.0, 500.0, 500.0), within.bounds)
        assertTrue(contentOnScreen(fitted, 1000.0, 2000.0, within.bounds, 1080, 1216))
        // A stroke out past the page and the screen's edge, or one whose box cannot be judged.
        val out = layerLists(listOf("notes"), listOf(item("a", "notes"), item("b", "notes").copy(bounds = Rect(900.0, 10.0, 2_000.0, 10.0)))) { it }
        assertFalse(contentOnScreen(fitted, 1000.0, 2000.0, out.bounds, 1080, 1216))
        val unknown = layerLists(listOf("notes"), listOf(item("a", "notes").copy(bounds = Rect(Double.NaN, 0.0, 1.0, 1.0)))) { it }
        assertNull(unknown.bounds)
        assertFalse(contentOnScreen(fitted, 1000.0, 2000.0, unknown.bounds, 1080, 1216))
        // An empty page: the page alone.
        assertTrue(contentOnScreen(fitted, 1000.0, 2000.0, layerLists(listOf("notes"), emptyList<RenderItem>()) { it }.bounds, 1080, 1216))
    }

    @Test fun the_page_s_soft_picture_is_no_larger_than_the_screen() {
        assertEquals(0.5, requireNotNull(overviewScale(2000.0, 1000.0, 1000, 1200)), 1e-12)
        assertEquals(1.2, requireNotNull(overviewScale(500.0, 1000.0, 1080, 1200)), 1e-12)
        val scale = requireNotNull(overviewScale(4032.0, 3024.0, 1080, 1216))
        assertTrue(4032 * scale <= 1080 + 1e-9 && 3024 * scale <= 1216 + 1e-9)
        assertNull(overviewScale(0.0, 100.0, 1080, 1216)); assertNull(overviewScale(100.0, 100.0, 0, 1216))
        assertNull(overviewScale(Double.NaN, 100.0, 1080, 1216))
    }

    @Test fun a_pan_moves_the_page_by_whole_pixels_and_carries_the_rest() {
        var rest = Point(0.0, 0.0)
        var moved = Point(0.0, 0.0)
        // Three fifths of a pixel at a time: the page moves at the first, third and fifth step, never by a fraction.
        repeat(5) {
            val (step, carried) = wholePixelPan(rest, 0.6, -0.6)
            assertEquals(step.x, kotlin.math.round(step.x), 0.0); assertEquals(step.y, kotlin.math.round(step.y), 0.0)
            assertTrue(kotlin.math.abs(carried.x) <= 0.5 && kotlin.math.abs(carried.y) <= 0.5)
            moved = Point(moved.x + step.x, moved.y + step.y); rest = carried
        }
        // Nothing is lost: moved and carried add up to what was asked.
        assertEquals(3.0, moved.x + rest.x, 1e-9); assertEquals(-3.0, moved.y + rest.y, 1e-9)
        assertEquals(Point(12.0, -4.0), wholePixelPan(Point(0.0, 0.0), 12.0, -4.0).first)
    }

    @Test fun pictures_are_drawn_at_the_view_when_the_gesture_ends_or_the_view_jumps_once() {
        // Fingers lifted: at once.
        assertTrue(viewSettlesNow(moving = false, wasMoving = true, sinceViewChangeMs = 0))
        // A single change of view (fit, a jump): at once; part of a movement without a contact: once frames stop.
        assertTrue(viewSettlesNow(moving = false, wasMoving = false, sinceViewChangeMs = -1))
        assertTrue(viewSettlesNow(moving = false, wasMoving = false, sinceViewChangeMs = PICTURE_STREAK_MS))
        assertFalse(viewSettlesNow(moving = false, wasMoving = false, sinceViewChangeMs = 8))
        // Still moving: never.
        assertFalse(viewSettlesNow(moving = true, wasMoving = true, sinceViewChangeMs = 5_000))
        assertFalse(viewSettlesNow(moving = true, wasMoving = false, sinceViewChangeMs = -1))
    }

    /** While the paired computer moves an object every frame wants another
     * picture. Making one for each would cost more than drawing the frame. */
    @Test fun a_picture_is_not_made_for_every_frame_of_a_movement() {
        val objects = PictureChange.Objects
        // A change after a quiet time, and the first there ever was: made at once.
        for (why in listOf(PictureChange.Missing, objects, PictureChange.Background, PictureChange.Layers, PictureChange.Area)) {
            assertTrue(why.name, pictureMadeNow(sameAsLastFrame = false, sinceChangeMs = -1, why = why))
            assertTrue(why.name, pictureMadeNow(sameAsLastFrame = false, sinceChangeMs = PICTURE_STREAK_MS, why = why))
        }
        assertTrue(pictureMadeNow(sameAsLastFrame = false, sinceChangeMs = 5_000, why = objects))
        // Another change right after the last: this frame is drawn over the pictures, or whole.
        assertFalse(pictureMadeNow(sameAsLastFrame = false, sinceChangeMs = 0, why = objects))
        assertFalse(pictureMadeNow(sameAsLastFrame = false, sinceChangeMs = PICTURE_STREAK_MS - 1, why = objects))
        // A resize starts a movement: drawn whole however long the page rested.
        for (why in listOf(PictureChange.View, PictureChange.Size)) {
            assertFalse(why.name, pictureMadeNow(sameAsLastFrame = false, sinceChangeMs = 5_000, why = why))
            assertTrue(why.name, pictureMadeNow(sameAsLastFrame = true, sinceChangeMs = 0, why = why))
        }
        // The movement has stopped: the next frame wants what the last one wanted, however soon it comes.
        assertTrue(pictureMadeNow(sameAsLastFrame = true, sinceChangeMs = 8, why = objects))
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
