@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

/** Receives the contours of a [Contours] in drawing order. */
public interface ContourVisitor {
    /** A contour of [vertexCount] vertices begins; its vertices follow. */
    public fun begin(vertexCount: Int)
    /** One vertex, in 1/256 document units. */
    public fun vertex(x: Long, y: Long)
    /** The contour that began last is complete. */
    public fun end()
}

/** Compound NONZERO fill in 1/256 D units; draw together once, never alpha-blend each contour.
 *
 * Held either as the vertex arrays it was made from or, for an item read from a
 * document, as the packed bytes the core sends ([packed]): the same vertices,
 * value for value. [visit] reads either form without converting it; [x], [y]
 * and [ends] unpack the packed form once, on first use, and keep the arrays.
 * Neither the arrays nor the bytes may be changed once handed over. Two
 * instances are equal when they hold the very same arrays or bytes. */
public class Contours private constructor(
    private val xs: LongArray?,
    private val ys: LongArray?,
    private val endsArray: UIntArray?,
    private val bytes: ByteArray?,
    /** Vertices over all contours. */
    public val vertexCount: Int,
    /** Number of contours. */
    public val contourCount: Int,
    /** The first vertex; 0 without vertices. */
    public val firstX: Long,
    public val firstY: Long,
) {
    public constructor(x: LongArray, y: LongArray, ends: UIntArray) :
        this(x, y, ends, null, x.size, ends.size, x.firstOrNull() ?: 0L, y.firstOrNull() ?: 0L)

    /** Whether this holds the packed bytes rather than arrays. */
    public val isPacked: Boolean get() = bytes != null

    private val unpacked: Triple<LongArray, LongArray, UIntArray> by lazy { unpack() }
    public val x: LongArray get() = xs ?: unpacked.first
    public val y: LongArray get() = ys ?: unpacked.second
    public val ends: UIntArray get() = endsArray ?: unpacked.third

    public fun copy(x: LongArray = this.x, y: LongArray = this.y, ends: UIntArray = this.ends): Contours = Contours(x, y, ends)

    /** Calls [visitor] for every contour and vertex in order. Array contours
     * must have as many x as y values and contour ends that rise to exactly
     * their vertex count. */
    public fun visit(visitor: ContourVisitor) {
        val packed = bytes
        if (packed == null) {
            val x = checkNotNull(xs); val y = checkNotNull(ys); val ends = checkNotNull(endsArray)
            require(x.size == y.size) { "contour arrays differ in length" }
            var first = 0
            for (endValue in ends) {
                require(endValue <= x.size.toUInt() && endValue.toInt() >= first) { "contour end out of order" }
                val end = endValue.toInt()
                visitor.begin(end - first)
                for (i in first until end) visitor.vertex(x[i], y[i])
                visitor.end()
                first = end
            }
            require(first == x.size) { "vertices after the last contour" }
            return
        }
        if (packed.isEmpty()) return
        val reader = PackedReader(packed)
        reader.header()
        var x = 0L; var y = 0L
        repeat(contourCount) {
            val count = reader.count()
            visitor.begin(count)
            repeat(count) {
                x += reader.signed(); y += reader.signed()
                visitor.vertex(x, y)
            }
            visitor.end()
        }
    }

    override fun equals(other: Any?): Boolean = other is Contours && xs === other.xs && ys === other.ys &&
        endsArray?.asIntArray() === other.endsArray?.asIntArray() && bytes === other.bytes
    override fun hashCode(): Int = ((xs.hashCode() * 31 + ys.hashCode()) * 31 + endsArray.hashCode()) * 31 + bytes.hashCode()
    override fun toString(): String = "Contours(vertices=$vertexCount, contours=$contourCount, packed=$isPacked)"

    private fun unpack(): Triple<LongArray, LongArray, UIntArray> {
        val outX = LongArray(vertexCount); val outY = LongArray(vertexCount); val outEnds = UIntArray(contourCount)
        var at = 0; var contour = 0
        visit(object : ContourVisitor {
            override fun begin(vertexCount: Int) {}
            override fun vertex(x: Long, y: Long) { outX[at] = x; outY[at] = y; at++ }
            override fun end() { outEnds[contour++] = at.toUInt() }
        })
        return Triple(outX, outY, outEnds)
    }

    public companion object {
        /** Packed contours as the core sends them (`PackedRenderItem.contours`):
         * empty, or LEB128 numbers giving the format version 1, the contour
         * count, the vertex count, then per contour its vertex count and per
         * vertex the zigzag-coded difference from the previous vertex in x and
         * in y, in wrapping 64-bit arithmetic. Checked completely here, so
         * [visit] cannot fail later; refuses malformed bytes with
         * [IllegalArgumentException]. The bytes are kept, not copied. */
        public fun packed(bytes: ByteArray): Contours {
            if (bytes.isEmpty()) return Contours(null, null, null, bytes, 0, 0, 0L, 0L)
            val reader = PackedReader(bytes)
            val (contours, vertices) = reader.header()
            var firstX = 0L; var firstY = 0L; var seen = 0L
            repeat(contours) {
                val count = reader.count()
                require(count <= vertices - seen) { "more vertices than announced" }
                repeat(count) {
                    val dx = reader.signed(); val dy = reader.signed()
                    // The first difference is from 0, 0: it is the first vertex.
                    if (seen == 0L) { firstX = dx; firstY = dy }
                    seen++
                }
            }
            require(seen == vertices.toLong() && reader.done()) { "packed contours do not add up" }
            return Contours(null, null, null, bytes, vertices, contours, firstX, firstY)
        }
    }
}

/** Reads the numbers of packed contours; every read checks its bounds. */
private class PackedReader(private val bytes: ByteArray) {
    private var at = 0
    fun done(): Boolean = at == bytes.size
    fun unsigned(): Long {
        var value = 0L; var shift = 0
        while (true) {
            require(at < bytes.size) { "packed contours end early" }
            val byte = bytes[at++].toInt()
            // The tenth byte may only carry the top bit of a 64-bit number.
            require(shift < 63 || (byte and 0xfe) == 0) { "number too long" }
            value = value or ((byte and 0x7f).toLong() shl shift)
            if (byte and 0x80 == 0) return value
            shift += 7
        }
    }
    fun signed(): Long { val value = unsigned(); return (value ushr 1) xor -(value and 1L) }
    /** A count that fits an array index. */
    fun count(): Int { val value = unsigned(); require(value in 0..Int.MAX_VALUE.toLong()) { "count too large" }; return value.toInt() }
    /** The version, then the contour and vertex counts. */
    fun header(): Pair<Int, Int> {
        require(unsigned() == 1L) { "unknown packed contour version" }
        val contours = count(); val vertices = count()
        return contours to vertices
    }
}
