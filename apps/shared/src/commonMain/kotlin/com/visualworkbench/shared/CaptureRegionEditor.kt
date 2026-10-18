package com.visualworkbench.shared

/** Local picker geometry only. Native picker_region validates the final physical
 * rectangle; this display controller cannot refresh a source or authorize IO. */
public class CaptureRegionEditor(private val map: (Camera, Boolean, List<Point>) -> List<Point>) {
    private data class Binding(val source: String, val width: UInt, val height: UInt, val camera: Camera)
    private var binding: Binding? = null
    private var down: Point? = null
    private var end: Point? = null
    public val edges: List<Double>?
        get() = down?.let { first -> end?.let { last -> listOf(first.x, first.y, last.x, last.y) } }
    public val contacting: Boolean get() = down != null && !complete
    private var complete: Boolean = false

    public fun bind(source: String, width: UInt, height: UInt, camera: Camera): Unit {
        require(source.isNotEmpty() && source.length <= 256 && width in 1u..32768u && height in 1u..32768u)
        val next = Binding(source, width, height, camera)
        if (next != binding) cancel()
        binding = next
    }
    public fun begin(physicalPoint: Point): Boolean {
        cancel()
        val point = framePoint(physicalPoint) ?: return false
        down = point; end = point
        return true
    }
    public fun move(physicalPoint: Point): Boolean {
        if (down == null || complete) return false
        val point = framePoint(physicalPoint) ?: return false
        end = point; return true
    }
    public fun finish(physicalPoint: Point): Boolean {
        if (!move(physicalPoint)) { cancel(); return false }
        val first = down ?: return false; val last = end ?: return false
        if (first.x == last.x || first.y == last.y) { cancel(); return false }
        complete = true; return true
    }
    public fun completedEdges(): List<Double>? = if (complete) edges else null
    public fun cancel(): Unit { down = null; end = null; complete = false }
    public fun detach(): Unit { cancel(); binding = null }
    private fun framePoint(value: Point): Point? {
        val current = binding ?: return null
        if (!value.x.isFinite() || !value.y.isFinite()) return null
        val result = try { map(current.camera, true, listOf(value)).singleOrNull() } catch (_: Exception) { null }
        return result?.takeIf { it.x.isFinite() && it.y.isFinite() && it.x >= 0 && it.y >= 0 &&
            it.x <= current.width.toDouble() && it.y <= current.height.toDouble() }
    }
}
