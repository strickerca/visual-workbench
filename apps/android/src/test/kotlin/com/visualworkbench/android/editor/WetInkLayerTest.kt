@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

/** When an ink frame may draw only the new stroke over the kept picture, and
 * which saved objects a magnifier pane may leave out. The drawing itself is
 * framework code and is judged on the device. */
class WetInkLayerTest {
    private fun item(id: String, layer: String, opacity: Double = 1.0, blend: String = "normal", shape: Shape = Shape.Stroke("pen")) =
        RenderItem(id, layer, opacity, blend, Rect(0.0, 0.0, 1.0, 1.0), Contours(longArrayOf(), longArrayOf(), uintArrayOf()),
            Transform(), ObjectStyle(0x203040ffu, 4.0), shape, false)
    private val layers = listOf("notes", "top")

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
