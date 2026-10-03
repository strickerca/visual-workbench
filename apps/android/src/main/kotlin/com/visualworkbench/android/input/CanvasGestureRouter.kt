package com.visualworkbench.android.input

import android.view.MotionEvent
import kotlin.math.atan2
import kotlin.math.hypot

/** Input policy only. Camera math and every document mutation remain in the core. */
internal interface CanvasGestureSink {
    fun beginTool(sample: PenSample, capabilities: StylusCapabilities)
    fun extendTool(samples: List<PenSample>)
    fun finishTool(samples: List<PenSample>)
    fun cancelTool()
    fun pan(dx: Double, dy: Double)
    fun pinch(previousX: Double, previousY: Double, nextX: Double, nextY: Double, scale: Double, radians: Double)
    fun undo()
    fun redo()
    fun contextMenu(x: Double, y: Double)
    fun hover(sample: PenSample?, capabilities: StylusCapabilities)
}

internal class CanvasGestureRouter(private val sink: CanvasGestureSink, private val slop: Double) {
    var drawWithFinger = false
    private var drawing: Int? = null
    private var fingerStroke = false
    private var lastFrame: PointerFrame? = null
    private var navigationStart: PointerFrame? = null
    private var maximumFingers = 0
    private var moved = false
    private var contextShown = false
    private var lastToolTime = 0L

    fun cancel() {
        if (drawing != null) sink.cancelTool()
        drawing = null; fingerStroke = false; lastFrame = null; navigationStart = null
        maximumFingers = 0; moved = false; contextShown = false
    }

    fun longPress(nowMs: Long) {
        val start = navigationStart ?: return
        if (drawing != null || maximumFingers != 1 || moved || contextShown || nowMs - start.timeMs < 500) return
        start.pointers.firstOrNull { it.tool == PointerTool.Finger }?.let {
            contextShown = true
            sink.contextMenu(it.x, it.y)
        }
    }

    fun accept(packet: PointerPacket) {
        val current = packet.frames.lastOrNull() ?: return
        if (packet.action in setOf(MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_EXIT)) {
            sink.hover(if (packet.action == MotionEvent.ACTION_HOVER_EXIT) null else current.pointers.firstOrNull {
                it.tool == PointerTool.Pen || it.tool == PointerTool.Eraser
            }, packet.capabilities)
            return
        }
        if (packet.canceled) {
            // FLAG_CANCELED on POINTER_UP applies to the pointer being lifted.
            if (packet.action != MotionEvent.ACTION_POINTER_UP || drawing == null || drawing == packet.actionPointer) cancel()
            else lastFrame = withoutLifted(current, packet)
            return
        }
        if (packet.action == MotionEvent.ACTION_DOWN) cancel()
        val fingers = current.pointers.filter { it.tool == PointerTool.Finger }
        val active = drawing
        if (active != null) {
            val sample = current.pointers.firstOrNull { it.pointerId == active }
            if (sample == null || sample.tool == PointerTool.Palm) { cancel(); return }
            if (fingerStroke && fingers.size > 1) {
                sink.cancelTool(); drawing = null; fingerStroke = false
                navigationStart = current; maximumFingers = fingers.size; moved = false; contextShown = false
                lastFrame = current
                return
            }
            if ((packet.action == MotionEvent.ACTION_UP || packet.action == MotionEvent.ACTION_POINTER_UP) && packet.actionPointer == active) {
                val samples = samples(packet, active)
                if (samples == null) sink.cancelTool() else sink.finishTool(samples)
                drawing = null; fingerStroke = false; lastFrame = null; navigationStart = null
                return
            }
            if (packet.action == MotionEvent.ACTION_MOVE) {
                val samples = samples(packet, active)
                if (samples == null) { cancel(); return }
                sink.extendTool(samples)
            }
            // A palm/finger beside an active pen is ignored, never interpreted as pan.
            return
        }
        val pressed = current.pointers.firstOrNull { it.pointerId == packet.actionPointer }
        if (packet.action == MotionEvent.ACTION_DOWN || packet.action == MotionEvent.ACTION_POINTER_DOWN) {
            if (pressed?.tool == PointerTool.Pen || pressed?.tool == PointerTool.Eraser ||
                (pressed?.tool == PointerTool.Finger && drawWithFinger && current.pointers.size == 1)) {
                val pen = checkNotNull(pressed)
                navigationStart = null; lastFrame = null; maximumFingers = 0
                drawing = pen.pointerId; fingerStroke = pen.tool == PointerTool.Finger; lastToolTime = pen.timeMs
                sink.beginTool(pen, packet.capabilities)
                return
            }
            if (fingers.isNotEmpty()) {
                if (navigationStart == null) { navigationStart = current; moved = false; contextShown = false }
                else if (fingers.size > maximumFingers) {
                    // Capture newly joined fingers so a pinch moving only its second finger still crosses slop.
                    navigationStart = current.copy(timeMs = checkNotNull(navigationStart).timeMs)
                }
                maximumFingers = maxOf(maximumFingers, fingers.size)
                lastFrame = current
            }
            return
        }
        if (packet.action == MotionEvent.ACTION_MOVE && fingers.isNotEmpty() && !contextShown) {
            maximumFingers = maxOf(maximumFingers, fingers.size)
            val prior = lastFrame?.pointers?.filter { it.tool == PointerTool.Finger } ?: emptyList()
            val pairs = fingers.mapNotNull { next -> prior.firstOrNull { it.pointerId == next.pointerId }?.let { it to next } }
            val origin = navigationStart
            if (origin != null && fingers.any { next -> origin.pointers.firstOrNull { it.pointerId == next.pointerId }
                    ?.let { hypot(next.x - it.x, next.y - it.y) > slop } == true }) moved = true
            if (moved && maximumFingers < 3) {
                if (pairs.size >= 2) {
                    val a = pairs[0]; val b = pairs[1]
                    val oldDistance = hypot(a.first.x - b.first.x, a.first.y - b.first.y)
                    val newDistance = hypot(a.second.x - b.second.x, a.second.y - b.second.y)
                    if (oldDistance > 1.0 && newDistance > 1.0) {
                        var angle = atan2(b.second.y - a.second.y, b.second.x - a.second.x) -
                            atan2(b.first.y - a.first.y, b.first.x - a.first.x)
                        while (angle > Math.PI) angle -= 2 * Math.PI
                        while (angle < -Math.PI) angle += 2 * Math.PI
                        sink.pinch((a.first.x + b.first.x) / 2, (a.first.y + b.first.y) / 2,
                            (a.second.x + b.second.x) / 2, (a.second.y + b.second.y) / 2,
                            newDistance / oldDistance, angle)
                    }
                } else if (pairs.size == 1 && maximumFingers == 1) {
                    sink.pan(pairs[0].second.x - pairs[0].first.x, pairs[0].second.y - pairs[0].first.y)
                }
            }
            lastFrame = current
            return
        }
        if (packet.action == MotionEvent.ACTION_POINTER_UP) { lastFrame = withoutLifted(current, packet); return }
        if (packet.action == MotionEvent.ACTION_UP) {
            val start = navigationStart
            if (start != null && !moved && !contextShown && current.timeMs - start.timeMs in 0..250) {
                if (maximumFingers == 2) sink.undo()
                if (maximumFingers == 3) sink.redo()
            }
            navigationStart = null; lastFrame = null; maximumFingers = 0
        }
    }

    private fun samples(packet: PointerPacket, pointer: Int): List<PenSample>? {
        val result = packet.frames.mapNotNull { frame -> frame.pointers.firstOrNull { it.pointerId == pointer } }
        if (result.isEmpty() || result.first().timeMs < lastToolTime || result.zipWithNext().any { (a, b) -> a.timeMs > b.timeMs }) return null
        lastToolTime = result.last().timeMs
        return result
    }

    private fun withoutLifted(frame: PointerFrame, packet: PointerPacket) =
        frame.copy(pointers = frame.pointers.filter { it.pointerId != packet.actionPointer })
}
