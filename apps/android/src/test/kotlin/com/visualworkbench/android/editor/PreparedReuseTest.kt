@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

/** Which prepared paths may be kept across a refresh. Path building itself is
 * framework code and is exercised on the device. */
class PreparedReuseTest {
    private fun contours() = Contours(longArrayOf(0, 256, 256), longArrayOf(0, 0, 256), uintArrayOf(3u))
    private fun stroke(id: String, contours: Contours = contours(), opacity: Double = 1.0) = RenderItem(id, "layer", opacity, "normal",
        Rect(0.0, 0.0, 1.0, 1.0), contours, Transform(), ObjectStyle(0x203040ffu, 4.0), Shape.Stroke("pen"), false)
    private fun rectangle(id: String) = RenderItem(id, "layer", 1.0, "normal", Rect(0.0, 0.0, 4.0, 3.0),
        Contours(longArrayOf(), longArrayOf(), uintArrayOf()), Transform(), ObjectStyle(0x203040ffu, 2.0), Shape.Rectangle(Rect(0.0, 0.0, 4.0, 3.0)), false)

    @Test fun nothing_earlier_or_another_object_is_always_rebuilt() {
        assertEquals(PreparedReuse.Rebuild, preparedReuse(null, stroke("a")))
        val a = stroke("a")
        assertEquals(PreparedReuse.Rebuild, preparedReuse(a, a.copy(objectId = "b")))
    }
    @Test fun the_same_instance_is_kept_and_an_equal_looking_fresh_read_is_not() {
        val a = stroke("a")
        assertEquals(PreparedReuse.Same, preparedReuse(a, a))
        // A fresh read of an unchanged stroke carries new arrays: without proof
        // from the core that it is unchanged, it is built again as before.
        assertEquals(PreparedReuse.Rebuild, preparedReuse(a, stroke("a")))
        val r = rectangle("r")
        assertEquals(PreparedReuse.Same, preparedReuse(r, r))
        assertEquals(PreparedReuse.Rebuild, preparedReuse(r, rectangle("r")))
    }
    @Test fun a_stroke_whose_layer_changed_keeps_its_path_but_takes_the_new_item() {
        val a = stroke("a")
        val dimmed = a.copy(layerOpacity = 0.5, layerBlend = "multiply", locked = true)
        assertSame(a.contours, dimmed.contours)
        assertEquals(PreparedReuse.SameOutline, preparedReuse(a, dimmed))
        // Shapes are stroked from their style and transform; they are never kept by outline.
        val r = rectangle("r")
        assertEquals(PreparedReuse.Rebuild, preparedReuse(r, r.copy(layerOpacity = 0.5)))
        // A stroke turned into something else is rebuilt even with the same arrays.
        assertEquals(PreparedReuse.Rebuild, preparedReuse(a, a.copy(shape = Shape.Rectangle(Rect(0.0, 0.0, 1.0, 1.0)))))
    }
}
