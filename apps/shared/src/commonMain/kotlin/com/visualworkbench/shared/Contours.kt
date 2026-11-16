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
 * value for value. [visit] reads either form without converting it and
 * allocates nothing; [x], [y] and [ends] unpack the packed form once, on first
 * use, and keep the arrays. The array form holds nothing else. Neither the
 * arrays nor the bytes may be changed once handed over. Two instances are
 * equal when they hold the very same arrays or bytes. */
public class Contours private constructor(
    private val xs: LongArray?,
    private val ys: LongArray?,
    private val endsArray: UIntArray?,
    private val bytes: ByteArray?,
    /** Packed only: where the first contour begins, after the header. */
    private val body: Int,
    /** Vertices over all contours. */
    public val vertexCount: Int,
    /** Number of contours. */
    public val contourCount: Int,
    /** The first vertex; 0 without vertices. */
    public val firstX: Long,
    public val firstY: Long,
) {
    public constructor(x: LongArray, y: LongArray, ends: UIntArray) :
        this(x, y, ends, null, 0, x.size, ends.size, x.firstOrNull() ?: 0L, y.firstOrNull() ?: 0L)

    /** Whether this holds the packed bytes rather than arrays. */
    public val isPacked: Boolean get() = bytes != null

    /** Packed only, set on first use of [x], [y] or [ends]. Two threads that
     * unpack at once both build equal arrays and either result is kept. */
    private var unpacked: Triple<LongArray, LongArray, UIntArray>? = null
    private fun arrays(): Triple<LongArray, LongArray, UIntArray> = unpacked ?: unpack().also { unpacked = it }
    public val x: LongArray get() = xs ?: arrays().first
    public val y: LongArray get() = ys ?: arrays().second
    public val ends: UIntArray get() = endsArray ?: arrays().third

    public fun copy(x: LongArray = this.x, y: LongArray = this.y, ends: UIntArray = this.ends): Contours = Contours(x, y, ends)

    /** Calls [visitor] for every contour and vertex in order; allocates
     * nothing. Array contours must have as many x as y values and contour ends
     * that rise to exactly their vertex count. */
    public fun visit(visitor: ContourVisitor) {
        val packed = bytes
        if (packed == null) {
            val x = checkNotNull(xs); val y = checkNotNull(ys); val ends = checkNotNull(endsArray)
            require(x.size == y.size) { "contour arrays differ in length" }
            var first = 0
            for (index in 0 until ends.size) {
                val endValue = ends[index]
                require(endValue <= x.size.toUInt() && endValue.toInt() >= first) { "contour end out of order" }
                val stop = endValue.toInt()
                visitor.begin(stop - first)
                for (i in first until stop) visitor.vertex(x[i], y[i])
                visitor.end()
                first = stop
            }
            require(first == x.size) { "vertices after the last contour" }
            return
        }
        var at = body; var px = 0L; var py = 0L
        repeat(contourCount) {
            var count = 0L
            at = readPacked(packed, at) { count = it }
            visitor.begin(count.toInt())
            repeat(count.toInt()) {
                var dx = 0L; var dy = 0L
                at = readPacked(packed, at) { dx = it }
                at = readPacked(packed, at) { dy = it }
                px += unzigzag(dx); py += unzigzag(dy)
                visitor.vertex(px, py)
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
        visit(object : ContourVisitor {
            var at = 0; var contour = 0
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
         * in y, in wrapping 64-bit arithmetic. Checked completely here, without
         * allocating, so [visit] cannot fail later; refuses malformed bytes
         * with [IllegalArgumentException]. The bytes are kept, not copied. */
        public fun packed(bytes: ByteArray): Contours {
            if (bytes.isEmpty()) return Contours(null, null, null, bytes, 0, 0, 0, 0L, 0L)
            var version = 0L; var contours = 0L; var vertices = 0L
            var at = readPacked(bytes, 0) { version = it }
            require(version == 1L) { "unknown packed contour version" }
            at = readPacked(bytes, at) { contours = it }
            at = readPacked(bytes, at) { vertices = it }
            require(contours in 0..Int.MAX_VALUE.toLong() && vertices in 0..Int.MAX_VALUE.toLong()) { "count too large" }
            val body = at
            var firstX = 0L; var firstY = 0L; var seen = 0L
            repeat(contours.toInt()) {
                var count = 0L
                at = readPacked(bytes, at) { count = it }
                require(count in 0..vertices - seen) { "more vertices than announced" }
                repeat(count.toInt()) {
                    var dx = 0L; var dy = 0L
                    at = readPacked(bytes, at) { dx = it }
                    at = readPacked(bytes, at) { dy = it }
                    // The first difference is from 0, 0: it is the first vertex.
                    if (seen == 0L) { firstX = unzigzag(dx); firstY = unzigzag(dy) }
                    seen++
                }
            }
            require(seen == vertices && at == bytes.size) { "packed contours do not add up" }
            return Contours(null, null, null, bytes, body, vertices.toInt(), contours.toInt(), firstX, firstY)
        }
    }
}

/** Reads the LEB128 number at [from] in [bytes], hands it to [value] and
 * returns where the next number starts. Every byte read is bounds-checked. */
private inline fun readPacked(bytes: ByteArray, from: Int, value: (Long) -> Unit): Int {
    var at = from; var result = 0L; var shift = 0
    while (true) {
        require(at < bytes.size) { "packed contours end early" }
        val byte = bytes[at++].toInt()
        // The tenth byte may only carry the top bit of a 64-bit number.
        require(shift < 63 || (byte and 0xfe) == 0) { "number too long" }
        result = result or ((byte and 0x7f).toLong() shl shift)
        if (byte and 0x80 == 0) { value(result); return at }
        shift += 7
    }
}
private fun unzigzag(value: Long): Long = (value ushr 1) xor -(value and 1L)
