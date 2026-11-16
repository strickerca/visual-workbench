@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import kotlin.test.*

/** The packed form the core sends gives back exactly the arrays it stands for. */
class ContoursTest {
    /** The layout written by the core, pinned in its own test of the same bytes. */
    private val twoTriangles = byteArrayOf(1, 2, 6,
        3, 0, 0, -128, 4, 0, -1, 3, -128, 4,
        3, -128, 8, -127, 4, -128, 4, 2, -1, 3, -128, 4)

    /** An encoder written from the documented layout, for values the core can send. */
    private fun pack(x: LongArray, y: LongArray, ends: UIntArray): ByteArray {
        val out = mutableListOf<Byte>()
        fun put(value: Long) { var v = value; while (v.toULong() >= 0x80uL) { out += (v.toInt() and 0x7f or 0x80).toByte(); v = v ushr 7 }; out += v.toByte() }
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
    private fun visited(contours: Contours): List<String> = buildList {
        contours.visit(object : ContourVisitor {
            override fun begin(vertexCount: Int) { add("begin $vertexCount") }
            override fun vertex(x: Long, y: Long) { add("$x,$y") }
            override fun end() { add("end") }
        })
    }

    @Test fun thePinnedLayoutReadsAsItsArrays() {
        val packed = Contours.packed(twoTriangles)
        assertTrue(packed.isPacked)
        assertEquals(6, packed.vertexCount); assertEquals(2, packed.contourCount)
        assertEquals(0L, packed.firstX); assertEquals(0L, packed.firstY)
        val arrays = Contours(longArrayOf(0, 256, 0, 512, 768, 512), longArrayOf(0, 0, 256, -1, 0, 256), uintArrayOf(3u, 6u))
        assertContentEquals(arrays.x, packed.x); assertContentEquals(arrays.y, packed.y)
        assertContentEquals(arrays.ends, packed.ends)
        assertEquals(visited(arrays), visited(packed))
        assertContentEquals(twoTriangles, pack(arrays.x, arrays.y, arrays.ends))
    }

    @Test fun everySixtyFourBitValueComesBackExactly() {
        val values = longArrayOf(Long.MIN_VALUE, Long.MIN_VALUE + 1, -(1L shl 40), -257, -1, 0, 1, 63, 64, 8191, 8192,
            (1_000_004_096L * 256), 1L shl 40, Long.MAX_VALUE - 1, Long.MAX_VALUE)
        val x = LongArray(values.size * 3) { values[(it * 7) % values.size] }
        val y = LongArray(values.size * 3) { values[(it * 11 + 3) % values.size] }
        val ends = UIntArray(values.size) { (it * 3 + 3).toUInt() }
        val packed = Contours.packed(pack(x, y, ends))
        assertContentEquals(x, packed.x); assertContentEquals(y, packed.y); assertContentEquals(ends, packed.ends)
        assertEquals(x[0], packed.firstX); assertEquals(y[0], packed.firstY)
        assertEquals(visited(Contours(x, y, ends)), visited(packed))
    }

    @Test fun emptyHoldsNothing() {
        val empty = Contours.packed(byteArrayOf())
        assertEquals(0, empty.vertexCount); assertEquals(0, empty.contourCount)
        assertTrue(empty.x.isEmpty() && empty.ends.isEmpty() && visited(empty).isEmpty())
    }

    @Test fun malformedBytesAreRefusedUpFront() {
        for (bytes in listOf(
            byteArrayOf(2, 0, 0), // unknown version
            twoTriangles.copyOf(twoTriangles.size - 1), // ends early
            twoTriangles + 0, // trailing byte
            byteArrayOf(1, 1, 4, 3, 0, 0, 0, 0, 0, 0), // fewer vertices than announced
            byteArrayOf(1, 1, 2, 3, 0, 0, 0, 0, 0, 0), // more vertices than announced
            byteArrayOf(1, 1, 1, 1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 2, 0), // a number past 64 bits
        )) assertFailsWith<IllegalArgumentException> { Contours.packed(bytes) }
    }

    @Test fun malformedArraysAreRefusedWhenVisited() {
        for (contours in listOf(
            Contours(longArrayOf(0, 1, 2), longArrayOf(0, 1), uintArrayOf(3u)),
            Contours(longArrayOf(0, 1, 2), longArrayOf(0, 1, 2), uintArrayOf(4u)),
            Contours(longArrayOf(0, 1, 2), longArrayOf(0, 1, 2), uintArrayOf(2u)),
            Contours(longArrayOf(0, 1, 2, 3), longArrayOf(0, 1, 2, 3), uintArrayOf(3u, 1u)),
        )) assertFailsWith<IllegalArgumentException> { visited(contours) }
    }

    @Test fun instancesAreEqualOnlyWhenTheyHoldTheSameData() {
        val x = longArrayOf(0, 256, 0); val y = longArrayOf(0, 0, 256); val ends = uintArrayOf(3u)
        assertEquals(Contours(x, y, ends), Contours(x, y, ends))
        assertNotEquals(Contours(x, y, ends), Contours(x.copyOf(), y, ends))
        assertEquals(Contours.packed(twoTriangles), Contours.packed(twoTriangles))
        assertNotEquals(Contours.packed(twoTriangles), Contours.packed(twoTriangles.copyOf()))
    }
}
