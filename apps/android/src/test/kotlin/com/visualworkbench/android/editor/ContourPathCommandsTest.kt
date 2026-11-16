@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

/** A stroke read from a document arrives packed; its path commands, origin
 * included, are exactly those of the same contours held as arrays. The
 * framework Path the commands go to is exercised on the device. */
class ContourPathCommandsTest {
    /** The packed layout the core sends, written from its description. */
    private fun pack(x: LongArray, y: LongArray, ends: UIntArray): ByteArray {
        val out = java.io.ByteArrayOutputStream()
        fun put(value: Long) { var v = value; while (v.toULong() >= 0x80uL) { out.write(v.toInt() and 0x7f or 0x80); v = v ushr 7 }; out.write(v.toInt()) }
        fun signed(value: Long) = put((value shl 1) xor (value shr 63))
        put(1); put(ends.size.toLong()); put(x.size.toLong())
        var first = 0; var px = 0L; var py = 0L
        for (end in ends) {
            put(end.toLong() - first)
            for (i in first until end.toInt()) { signed(x[i] - px); signed(y[i] - py); px = x[i]; py = y[i] }
            first = end.toInt()
        }
        return out.toByteArray()
    }
    private fun stroke(contours: Contours) = RenderItem("s", "layer", 1.0, "normal", Rect(0.0, 0.0, 1.0, 1.0), contours,
        Transform(), ObjectStyle(0x51c7c2ffu, 6.0), Shape.Stroke("pen"), false)
    /** Every command with its exact float bits. */
    /** The commands a [ContourPath] issues to its framework path, recorded
     * with their exact float bits through the same [ContourCommands]. */
    private class Recorder(override val originX: Double, override val originY: Double) : ContourCommands {
        override var started = false
        val commands = mutableListOf<String>()
        override fun moveTo(x: Float, y: Float) { commands += "M ${x.toRawBits()} ${y.toRawBits()}" }
        override fun lineTo(x: Float, y: Float) { commands += "L ${x.toRawBits()} ${y.toRawBits()}" }
        override fun close() { commands += "Z" }
    }
    private fun commands(item: RenderItem): List<String> {
        val origin = EditorDrawing.objectOrigin(item)
        return Recorder(origin.x, origin.y).also { item.contours.visit(it) }.commands
    }

    @Test fun packedAndArrayContoursGiveTheSamePathCommandsAndOrigin() {
        // A circle of 16 vertices and a quad far from the origin, where the
        // f64 origin subtraction decides the float that is drawn.
        val x = LongArray(16) { (1_234_567.0 * 256 + 900 * kotlin.math.cos(it * Math.PI / 8)).toLong() } + longArrayOf(-3, 300_000_001, 300_000_001, -3)
        val y = LongArray(16) { (7_654_321.0 * 256 + 900 * kotlin.math.sin(it * Math.PI / 8)).toLong() } + longArrayOf(-7, -7, 99, 99)
        val ends = uintArrayOf(16u, 20u)
        val arrays = stroke(Contours(x, y, ends)); val packed = stroke(Contours.packed(pack(x, y, ends)))
        assertTrue(packed.contours.isPacked)
        assertEquals(EditorDrawing.objectOrigin(arrays), EditorDrawing.objectOrigin(packed))
        assertEquals(Point(x[0] / 256.0, y[0] / 256.0), EditorDrawing.objectOrigin(packed))
        val expected = commands(arrays)
        assertEquals(1 + 15 + 1 + 1 + 3 + 1, expected.size)
        assertEquals(expected, commands(packed))
    }

    @Test fun aContourOfFewerThanThreeVerticesIsRefusedInEitherForm() {
        val x = longArrayOf(0, 256); val y = longArrayOf(0, 0); val ends = uintArrayOf(2u)
        for (contours in listOf(Contours(x, y, ends), Contours.packed(pack(x, y, ends))))
            assertThrows(IllegalArgumentException::class.java) { commands(stroke(contours)) }
    }
}
