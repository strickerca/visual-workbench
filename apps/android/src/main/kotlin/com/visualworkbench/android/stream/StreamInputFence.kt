package com.visualworkbench.android.stream

import com.visualworkbench.android.input.*
import com.visualworkbench.shared.Shape

/** Adapter for the existing CanvasGestureRouter, not a second recognizer. Only
 * its real, root-admitted pen DOWN stops live draws. Keep a bounded first gesture
 * while the exact frozen draft opens; cancellation never replays that gesture. */
internal class StreamInputFence(
    private val admitDown: (PenSample, StylusCapabilities) -> Boolean,
    private val freeze: () -> Unit,
) : CanvasGestureSink {
    private class Contact(val down: PenSample, val capabilities: StylusCapabilities) {
        val samples = mutableListOf<PenSample>()
        var finished = false
    }
    private var child: CanvasGestureSink? = null
    private var contact: Contact? = null
    private var heldFirst = false
    private var activeContact = false
    private var opening = false
    private var blocked = false

    fun attach(sink: CanvasGestureSink) {
        check(child == null)
        child = sink; opening = false
        val pending = contact; contact = null
        heldFirst = false
        activeContact = !blocked && pending != null && !pending.finished
        if (!blocked && pending != null) {
            sink.beginTool(pending.down, pending.capabilities)
            val chunks = pending.samples.chunked(512)
            if (pending.finished) {
                chunks.dropLast(1).forEach(sink::extendTool)
                sink.finishTool(chunks.lastOrNull() ?: emptyList())
            } else chunks.forEach(sink::extendTool)
        }
    }
    /** Lifecycle interruption also drops a completed but not-yet-consumed stroke. */
    fun interrupt() { contact = null; activeContact = false; child?.cancelTool() }
    fun detach() { interrupt(); child = null; opening = false; heldFirst = false; blocked = false }
    fun block(value: Boolean) { if (value) interrupt(); blocked = value }
    override fun beginTool(sample: PenSample, capabilities: StylusCapabilities) {
        activeContact = false
        if (blocked) return
        child?.let { activeContact = true; it.beginTool(sample, capabilities); return }
        // A subsequent physical stroke can arrive while the editor factory is
        // suspended. It must neither replace nor append to the first one, even
        // when Android reuses the pointer ID. Cancellation leaves a tombstone.
        if (heldFirst) return
        if (sample.tool != PointerTool.Pen || !admitDown(sample, capabilities)) return
        heldFirst = true; activeContact = true
        contact = Contact(sample, capabilities)
        if (!opening) { opening = true; freeze() }
    }
    private fun append(samples: List<PenSample>): Boolean {
        val pending = contact?.takeIf { activeContact && !it.finished } ?: return false
        if (samples.size > 4096 - pending.samples.size || samples.any {
                it.pointerId != pending.down.pointerId || it.tool != PointerTool.Pen || it.timeMs < pending.down.timeMs }) {
            contact = null; activeContact = false; return false
        }
        pending.samples += samples
        return true
    }
    override fun extendTool(samples: List<PenSample>) {
        if (blocked) return
        child?.let { if (activeContact) it.extendTool(samples); return }
        append(samples)
    }
    override fun finishTool(samples: List<PenSample>) {
        if (blocked) return
        child?.let { if (activeContact) { activeContact = false; it.finishTool(samples) }; return }
        if (append(samples)) contact?.finished = true
        activeContact = false
    }
    override fun cancelTool() {
        if (activeContact) contact = null
        activeContact = false; child?.cancelTool()
    }
    override fun navigationContact(active: Boolean) { if (!blocked) child?.navigationContact(active) }
    override fun pan(dx: Double, dy: Double) { if (!blocked) child?.pan(dx, dy) }
    override fun pinch(previousX: Double, previousY: Double, nextX: Double, nextY: Double, scale: Double, radians: Double) {
        if (!blocked) child?.pinch(previousX, previousY, nextX, nextY, scale, radians)
    }
    override fun undo() { if (!blocked) child?.undo() }
    override fun redo() { if (!blocked) child?.redo() }
    override fun contextMenu(x: Double, y: Double) { if (!blocked) child?.contextMenu(x, y) }
    override fun hover(sample: PenSample?, capabilities: StylusCapabilities) { if (!blocked) child?.hover(sample, capabilities) }
}

/** Native Save excludes raster dependencies and vector eraser strokes. Plain
 * markers are allowed; the root must not attach semantic element references. */
internal fun streamAnnotationShapeSupported(shape: Shape): Boolean = when (shape) {
    is Shape.Result, Shape.Adjustment -> false
    is Shape.Stroke -> shape.family != "vector_eraser"
    else -> true
}
