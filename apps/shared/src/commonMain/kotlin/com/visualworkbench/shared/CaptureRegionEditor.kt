package com.visualworkbench.shared

import kotlinx.coroutines.CancellationException

/** UI-dispatcher-owned local picker geometry. Input and camera viewport use the
 * same physical display pixels; output edges remain unrounded original frame
 * pixels. Native crop admission owns host origin and final pixel-edge snapping.
 * This helper cannot refresh a source or authorize capture IO. */
public class CaptureRegionEditor(private val map: (Camera, Boolean, List<Point>) -> List<Point>) {
    private data class Binding(val source: String, val width: UInt, val height: UInt, val camera: Camera)
    private var binding: Binding? = null
    private var inputEpoch: Any = Any()
    private var down: Point? = null
    private var end: Point? = null
    public val edges: List<Double>?
        get() = down?.let { first -> end?.let { last -> listOf(first.x, first.y, last.x, last.y) } }
    public val contacting: Boolean get() = down != null && !complete
    private var complete: Boolean = false

    public fun bind(source: String, width: UInt, height: UInt, camera: Camera): Unit {
        if (!validBinding(source, width, height, camera)) {
            // A refused replacement must not leave the old source selectable.
            detach()
            throw IllegalArgumentException("Invalid capture picker binding")
        }
        val next = Binding(source, width, height, camera)
        if (next != binding) { cancel(); binding = next }
    }
    public fun begin(physicalPoint: Point): Boolean {
        cancel()
        val point = framePoint(physicalPoint) ?: return false
        down = point; end = point
        return true
    }
    public fun move(physicalPoint: Point): Boolean {
        if (down == null || complete) return false
        val epoch = inputEpoch
        val point = framePoint(physicalPoint)
        if (point == null) {
            // Do not erase a replacement gesture created by a reentrant mapper.
            if (inputEpoch === epoch) cancel()
            return false
        }
        end = point
        return true
    }
    public fun finish(physicalPoint: Point): Boolean {
        if (!move(physicalPoint)) return false
        val first = down ?: return false; val last = end ?: return false
        if (first.x == last.x || first.y == last.y) { cancel(); return false }
        complete = true
        return true
    }
    public fun completedEdges(): List<Double>? = if (complete) edges else null
    public fun cancel(): Unit { inputEpoch = Any(); down = null; end = null; complete = false }
    public fun detach(): Unit { cancel(); binding = null }
    private fun framePoint(value: Point): Point? {
        val current = binding ?: return null
        val epoch = inputEpoch
        if (!value.x.isFinite() || !value.y.isFinite() || value.x < 0.0 || value.y < 0.0 ||
            value.x > current.camera.viewportWidth || value.y > current.camera.viewportHeight) return null
        val result = try { map(current.camera, true, listOf(value)).singleOrNull() }
        catch (error: CancellationException) {
            if (inputEpoch === epoch) cancel()
            throw error
        } catch (_: Exception) { null }
        if (binding !== current || inputEpoch !== epoch) return null
        return result?.takeIf { it.x.isFinite() && it.y.isFinite() && it.x >= 0 && it.y >= 0 &&
            it.x <= current.width.toDouble() && it.y <= current.height.toDouble() }
    }
    private fun validBinding(source: String, width: UInt, height: UInt, camera: Camera): Boolean {
        if (source.isEmpty() || source.length > 256 || width !in 1u..32768u || height !in 1u..32768u ||
            camera.rotation != 0.0 || !camera.scale.isFinite() || camera.scale <= 0.0 ||
            !(1.0 / camera.scale).isFinite() || !camera.center.x.isFinite() || !camera.center.y.isFinite() ||
            !camera.viewportWidth.isFinite() || !camera.viewportHeight.isFinite() ||
            camera.viewportWidth <= 0.0 || camera.viewportHeight <= 0.0) return false
        val left = camera.viewportWidth / 2 - camera.center.x * camera.scale
        val top = camera.viewportHeight / 2 - camera.center.y * camera.scale
        val right = left + width.toDouble() * camera.scale
        val bottom = top + height.toDouble() * camera.scale
        return listOf(left, top, right, bottom).all(Double::isFinite) && right > left && bottom > top
    }
}
