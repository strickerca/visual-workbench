@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel

internal data class DesktopSample(val point: Point, val uptimeMs: Long, val pressure: Float = 1f)

/** Raw desktop pointer input only. There is deliberately no prediction input.
 * Bounded intake fails by cancelling the whole gesture, never by inventing a
 * line between silently discarded samples. Mouse pressure is explicitly 1. */
internal class DesktopInkRun(
    val options: StrokeOptions,
    val binding: RenderBinding,
    val camera: Camera,
    val layer: LayerInfo,
    val attachment: LiveAttachment?,
    private val first: DesktopSample,
) {
    private val input = Channel<DesktopSample>(512)
    private var supplied = 0
    private var lastTime = first.uptimeMs
    private var native: WorkbenchStroke? = null
    var cancelled = false; private set
    var finished = false; private set
    var committed = false; private set
    init { check(offer(first)) }

    fun offer(sample: DesktopSample): Boolean {
        if (cancelled || finished) return false
        if (++supplied > 100_000 || sample.uptimeMs < lastTime || sample.uptimeMs < first.uptimeMs ||
            sample.uptimeMs - first.uptimeMs > UInt.MAX_VALUE.toLong() || !sample.pressure.isFinite() ||
            sample.pressure !in 0f..1f || !sample.point.x.isFinite() || !sample.point.y.isFinite() ||
            input.trySend(sample).isFailure) { cancel(); return false }
        lastTime = sample.uptimeMs
        return true
    }

    fun finish() { if (!cancelled) { finished = true; input.close() } }
    fun cancel() {
        cancelled = true
        input.close()
        try { native?.cancel() }
        catch (error: CoreFailure) { if (error.kind !in setOf(CoreFailureKind.Closed, CoreFailureKind.Cancelled)) throw error }
    }

    suspend fun execute(
        project: WorkbenchProject,
        publish: (RenderItem) -> Unit,
        durable: suspend (ProjectInfo) -> Unit,
        finishPreview: suspend (Boolean) -> Unit,
    ) {
        var stroke: WorkbenchStroke? = null
        try {
            if (cancelled) return
            stroke = project.beginStroke(options)
            native = stroke
            if (cancelled) { stroke.cancel(); return }
            var sequence = 1uL
            var accepted = 0u
            var polygons = 0uL
            var contours = Contours(longArrayOf(), longArrayOf(), uintArrayOf())
            for (sample in input) {
                if (cancelled) break
                val samples = mutableListOf(sample)
                while (samples.size < 512) samples += input.tryReceive().getOrNull() ?: break
                val batch = SampleBatch(sequence, samples.map { it.point.x }.toDoubleArray(), samples.map { it.point.y }.toDoubleArray(),
                    samples.map { (it.uptimeMs - first.uptimeMs).toUInt() }.toUIntArray(), samples.map { it.pressure }.toFloatArray())
                val update = append(stroke, batch)
                require(update.sequence == sequence && update.firstPolygon == polygons && update.sampleCount == accepted.toULong() + samples.size.toULong())
                if (cancelled) { stroke.cancel(); break }
                // Only append's accepted batch may enter the volatile channel.
                try { attachment?.link?.streamStroke(stroke, batch, accepted) } catch (_: SessionFailure) { }
                accepted += samples.size.toUInt(); sequence++
                polygons += update.contours.ends.size.toULong()
                contours = appendContours(contours, update.contours)
                if (!cancelled) publish(RenderItem(options.objectId, layer.id, layer.opacity, layer.blend,
                    contourBounds(contours), contours, Transform(), ObjectStyle(options.rgba, options.width), Shape.Stroke(options.family), false))
                // At most 112 accepted preview batches per second, including a
                // backlogged pointer source. Durable samples are never skipped.
                delay(9)
            }
            if (finished && !cancelled && accepted > 0u) {
                val receipt = stroke.commit()
                committed = true
                // The receipt owns local wet-to-canonical bookkeeping even if
                // volatile closure fails or cancellation follows acceptance.
                try { withContext(NonCancellable) { durable(receipt) } }
                finally { finishPreview(false) }
            } else stroke.cancel()
        } finally {
            withContext(NonCancellable) {
                try {
                    if (!committed) {
                        try { stroke?.cancel() } catch (error: CoreFailure) { if (error.kind !in setOf(CoreFailureKind.Closed, CoreFailureKind.Cancelled)) throw error }
                        finishPreview(true)
                    }
                } finally {
                    try { stroke?.dispose() }
                    finally { native = null; input.cancel() }
                }
            }
        }
    }
}

private suspend fun append(stroke: WorkbenchStroke, batch: SampleBatch): InkUpdate {
    repeat(4) { attempt ->
        try { return stroke.append(batch) }
        catch (error: CoreFailure) {
            if (error.kind != CoreFailureKind.Backpressure || attempt == 3) throw error
            delay(4L shl attempt)
        }
    }
    error("Unreachable bounded append")
}

internal fun appendContours(before: Contours, added: Contours): Contours {
    require(added.x.size == added.y.size)
    val total = before.x.size.toLong() + added.x.size
    require(total <= 1_048_576) { "The live stroke geometry limit was reached; the gesture was cancelled." }
    var previous = 0u
    for (end in added.ends) { require(end > previous && end <= added.x.size.toUInt()); previous = end }
    require(previous == added.x.size.toUInt())
    val offset = before.x.size.toUInt()
    return Contours(before.x + added.x, before.y + added.y, before.ends + added.ends.map { it + offset }.toUIntArray())
}

private fun contourBounds(contours: Contours): Rect {
    if (contours.x.isEmpty()) return Rect(0.0, 0.0, 0.0, 0.0)
    val x = contours.x.min().toDouble() / 256; val y = contours.y.min().toDouble() / 256
    return Rect(x, y, contours.x.max().toDouble() / 256 - x, contours.y.max().toDouble() / 256 - y)
}
