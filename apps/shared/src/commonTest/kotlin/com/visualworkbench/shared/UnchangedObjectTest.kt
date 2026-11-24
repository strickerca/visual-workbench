@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/** An object that is not a pen stroke is sent whole at every read; it counts as
 * unchanged, and keeps the caller's instance, only when every field is equal. */
class UnchangedObjectTest {
    private fun empty() = Contours(longArrayOf(), longArrayOf(), uintArrayOf())
    private fun item(shape: Shape, id: String = "o", contours: Contours = empty()) = RenderItem(id, "layer", 1.0, "normal", Rect(1.0, 2.0, 30.0, 40.0),
        contours, Transform(e = 3.0), ObjectStyle(0x203040ffu, 2.0, fill = 0x8090a0ffu), shape, false)

    @Test fun every_kind_but_a_stroke_read_again_alike_is_unchanged() {
        val shapes = listOf(Shape.Rectangle(Rect(1.0, 2.0, 30.0, 40.0)), Shape.Ellipse(Rect(1.0, 2.0, 30.0, 40.0)),
            Shape.Line(listOf(Point(1.0, 2.0), Point(30.0, 40.0))), Shape.Arrow(listOf(Point(1.0, 2.0), Point(30.0, 40.0))),
            Shape.Polygon(listOf(Point(1.0, 2.0), Point(30.0, 40.0), Point(5.0, 40.0)), true),
            Shape.Text(Point(1.0, 2.0), "Text", "Inter", 12.0, listOf(Outline.Move(0f, 0f), Outline.Line(4f, 0f), Outline.Close)),
            Shape.Marker(3u, Point(1.0, 2.0), Rect(0.0, 0.0, 9.0, 9.0)), Shape.Guide(Rect(1.0, 2.0, 30.0, 40.0)))
        // Two reads: equal values in other instances, contours included.
        for (shape in shapes) assertTrue(unchangedObject(item(shape), item(shape)), shape.toString())
    }
    @Test fun any_field_that_differs_is_a_change() {
        val shape = Shape.Rectangle(Rect(1.0, 2.0, 30.0, 40.0))
        val earlier = item(shape)
        val changed = listOf(earlier.copy(objectId = "other"), earlier.copy(layerId = "other"), earlier.copy(layerOpacity = 0.5),
            earlier.copy(layerBlend = "multiply"), earlier.copy(locked = true), earlier.copy(bounds = Rect(1.0, 2.0, 30.0, 41.0)),
            earlier.copy(transform = Transform(e = 4.0)), earlier.copy(style = earlier.style.copy(rgba = 0xff0000ffu)),
            earlier.copy(style = earlier.style.copy(fill = null)), earlier.copy(style = earlier.style.copy(width = 3.0)),
            earlier.copy(style = earlier.style.copy(screenConstantWidth = true)), earlier.copy(shape = Shape.Rectangle(Rect(1.0, 2.0, 31.0, 40.0))),
            earlier.copy(shape = Shape.Ellipse(Rect(1.0, 2.0, 30.0, 40.0))))
        for (other in changed) assertFalse(unchangedObject(earlier, other), other.toString())
        // A marker renumbered or moved, a text edited.
        val marker = item(Shape.Marker(3u, Point(1.0, 2.0), null))
        assertFalse(unchangedObject(marker, marker.copy(shape = Shape.Marker(4u, Point(1.0, 2.0), null))))
        assertFalse(unchangedObject(marker, marker.copy(shape = Shape.Marker(3u, Point(1.0, 2.5), null))))
        val text = item(Shape.Text(Point(1.0, 2.0), "Text", "Inter", 12.0))
        assertFalse(unchangedObject(text, text.copy(shape = Shape.Text(Point(1.0, 2.0), "Test", "Inter", 12.0))))
        // An opacity that cannot be compared is never taken for the same.
        assertFalse(unchangedObject(earlier.copy(layerOpacity = Double.NaN), earlier.copy(layerOpacity = Double.NaN)))
    }
    @Test fun strokes_and_anything_with_contours_are_never_taken_for_unchanged_here() {
        val stroke = item(Shape.Stroke("pen"))
        assertFalse(unchangedObject(stroke, stroke))
        val outlined = item(Shape.Polygon(emptyList(), true), contours = Contours(longArrayOf(0, 256, 0), longArrayOf(0, 0, 256), uintArrayOf(3u)))
        assertFalse(unchangedObject(outlined, outlined))
        assertFalse(unchangedObject(item(Shape.Polygon(emptyList(), true)), outlined))
    }
}
