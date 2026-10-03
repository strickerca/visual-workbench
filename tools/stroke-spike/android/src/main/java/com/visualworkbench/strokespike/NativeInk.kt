package com.visualworkbench.strokespike

import android.graphics.Path

/** Primitive-only JNI boundary. Handles are registry IDs, never native addresses. */
internal object NativeInk {
    init { System.loadLibrary("vw_stroke_jni") }
    external fun begin(family: Int, width: Double, stabilization: Double): Long
    external fun append(handle: Long, x: Double, y: Double, timeMs: Long, pressure: Double): Int
    external fun preview(handle: Long, x: Double, y: Double, timeMs: Long, pressure: Double): Long
    external fun polygons(handle: Long): Int
    external fun vertices(handle: Long, polygon: Int): Int
    external fun coordinate(handle: Long, polygon: Int, vertex: Int, axis: Int): Double
    external fun finish(handle: Long): Int
    external fun hashWord(handle: Long, word: Int): Long
    external fun release(handle: Long): Int
    external fun liveHandles(): Int

    fun appendContours(path: Path, handle: Long, from: Int = 0): Int {
        val count = polygons(handle)
        check(count in from..100_000) { "Native stroke geometry rejected" }
        path.fillType = Path.FillType.WINDING
        for (polygon in from until count) {
            val size = vertices(handle, polygon)
            check(size in 3..512) { "Native contour rejected" }
            for (vertex in 0 until size) {
                val x = coordinate(handle, polygon, vertex, 0)
                val y = coordinate(handle, polygon, vertex, 1)
                check(x.isFinite() && y.isFinite()) { "Native point rejected" }
                if (vertex == 0) path.moveTo(x.toFloat(), y.toFloat()) else path.lineTo(x.toFloat(), y.toFloat())
            }
            path.close()
        }
        return count
    }

    fun hash(handle: Long): String = (0..3).joinToString("") { index ->
        java.lang.Long.toUnsignedString(hashWord(handle, index), 16).padStart(16, '0')
    }
}
