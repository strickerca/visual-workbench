package com.visualworkbench.strokespike

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.PorterDuff
import android.os.Handler
import android.os.Looper
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.graphics.lowlatency.CanvasFrontBufferedRenderer
import androidx.graphics.surface.SurfaceControlCompat
import androidx.input.motionprediction.MotionEventPredictor

/** Core-generated contours are filled once with NONZERO winding, including overlaps. */
internal class RustInkSurface(context: Context, private val metrics: StrokeMetrics) : SurfaceView(context) {
    private data class WetSnapshot(val path: Path, val inputMs: Long, val version: Int)
    private data class DrySnapshot(val generation: Any, val paths: List<Path>, val inputMs: Long?)
    // Queued requests share this slot, not a growing collection of full Path copies.
    // Every published Path is immutable; a callback takes one local snapshot.
    private class StrokeSlot(val generation: Any) {
        var latest: WetSnapshot? = null
        var version = 0
        var lastDrawnVersion = 0
        var queued = false
        var canceled = false
    }
    private sealed interface Frame {
        val stroke: StrokeSlot
        data class Wet(override val stroke: StrokeSlot) : Frame
        data class Commit(val dry: DrySnapshot, override val stroke: StrokeSlot) : Frame
        data class Cancel(override val stroke: StrokeSlot) : Frame
    }
    private class RendererSession(val generation: Any, var dry: DrySnapshot) {
        var closed = false
        var frontStroke: StrokeSlot? = null
        var lastDryMetric: DrySnapshot? = null
    }
    private val renderLock = Any()
    private val mainHandler = Handler(Looper.getMainLooper())
    private var renderer: CanvasFrontBufferedRenderer<Frame>? = null
    private var rendererSession: RendererSession? = null
    private var rendererRetiring = false
    private var surfaceWidth = 0
    private var surfaceHeight = 0
    private var generation = Any()
    private var latestDry = DrySnapshot(generation, emptyList(), null)
    private var activeSlot: StrokeSlot? = null
    private var predictor = MotionEventPredictor.newInstance(this)
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.rgb(24, 98, 121); style = Paint.Style.FILL }
    private val completed = mutableListOf<Path>()
    private var activePath = Path()
    private var handle = 0L
    private var pointerId = -1
    private var startMs = 0L
    private var lastMs = 0L
    private var samples = 0
    private var polygonCount = 0
    private var started = 0
    var lastWetHash = ""
        private set
    var lastDryHash = ""
        private set
    var committedCount = 0
        private set
    var canceledCount = 0
        private set
    var lastFailure: String? = null
        private set

    init {
        holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) { ensureRenderer() }

            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
                val hadSurface = surfaceWidth > 0 && surfaceHeight > 0
                val resized = width != surfaceWidth || height != surfaceHeight
                surfaceWidth = width
                surfaceHeight = height
                if (resized && hadSurface) {
                    cancelActive()
                    renewGeneration()
                }
                ensureRenderer()
            }

            override fun surfaceDestroyed(holder: SurfaceHolder) {
                surfaceWidth = 0
                surfaceHeight = 0
                cancelActive()
                renewGeneration()
            }
        })
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        ensureRenderer()
    }

    private fun ensureRenderer() {
        if (!isAttachedToWindow || !holder.surface.isValid || surfaceWidth <= 0 || surfaceHeight <= 0 ||
            width <= 0 || height <= 0 || renderer != null || rendererRetiring) return
        val session = RendererSession(generation, latestDry)
        rendererSession = session
        renderer = CanvasFrontBufferedRenderer(this, object : CanvasFrontBufferedRenderer.Callback<Frame> {
            override fun onDrawFrontBufferedLayer(canvas: Canvas, bufferWidth: Int, bufferHeight: Int, param: Frame) {
                val slot = param.stroke
                if (param is Frame.Cancel) {
                    val clear = synchronized(renderLock) {
                        if (session.closed || slot.generation !== session.generation) false
                        else { session.frontStroke = slot; true }
                    }
                    if (clear) canvas.drawColor(Color.TRANSPARENT, PorterDuff.Mode.CLEAR)
                    return
                }
                val snapshot = synchronized(renderLock) {
                    slot.queued = false
                    val latest = slot.latest
                    if (session.closed || slot.canceled || slot.generation !== session.generation ||
                        latest == null || latest.version <= slot.lastDrawnVersion) null
                    else {
                        slot.lastDrawnVersion = latest.version
                        session.frontStroke = slot
                        latest
                    }
                } ?: return
                canvas.drawColor(Color.TRANSPARENT, PorterDuff.Mode.CLEAR)
                canvas.drawPath(snapshot.path, paint)
                synchronized(renderLock) {
                    if (!session.closed && !slot.canceled) {
                        metrics.record("rust", "front_buffer_draw_callback", snapshot.inputMs)
                    }
                }
            }

            override fun onDrawMultiBufferedLayer(canvas: Canvas, bufferWidth: Int, bufferHeight: Int, params: Collection<Frame>) {
                val snapshot = synchronized(renderLock) {
                    if (session.closed) null else {
                        for (param in params) {
                            if (param is Frame.Commit && param.dry.generation === session.generation) {
                                session.dry = param.dry
                            }
                        }
                        session.dry
                    }
                } ?: return
                canvas.drawColor(Color.WHITE, PorterDuff.Mode.SRC)
                for (path in snapshot.paths) canvas.drawPath(path, paint)
                synchronized(renderLock) {
                    if (!session.closed && session.lastDryMetric !== snapshot) {
                        session.lastDryMetric = snapshot
                        snapshot.inputMs?.let { metrics.record("rust", "multi_buffer_draw_callback", it) }
                    }
                }
            }

            override fun onFrontBufferedLayerRenderComplete(
                frontBufferedLayerSurfaceControl: SurfaceControlCompat,
                transaction: SurfaceControlCompat.Transaction,
            ) {
                synchronized(renderLock) {
                    if (session.closed || session.frontStroke?.canceled == true) {
                        transaction.setVisibility(frontBufferedLayerSurfaceControl, false)
                    }
                }
            }

            override fun onMultiBufferedLayerRenderComplete(
                frontBufferedLayerSurfaceControl: SurfaceControlCompat,
                multiBufferedLayerSurfaceControl: SurfaceControlCompat,
                transaction: SurfaceControlCompat.Transaction,
            ) {
                synchronized(renderLock) {
                    if (session.closed) {
                        transaction.setVisibility(frontBufferedLayerSurfaceControl, false)
                        transaction.setVisibility(multiBufferedLayerSurfaceControl, false)
                    }
                }
            }
        })
        activeSlot?.let(::enqueueWetIfNeeded)
    }

    private fun enqueueWetIfNeeded(slot: StrokeSlot) {
        val currentRenderer = renderer ?: return
        val session = rendererSession ?: return
        val enqueue = synchronized(renderLock) {
            val latest = slot.latest
            if (session.closed || slot.canceled || slot.generation !== session.generation ||
                latest == null || latest.version <= slot.lastDrawnVersion || slot.queued) false
            else { slot.queued = true; true }
        }
        if (enqueue) currentRenderer.renderFrontBufferedLayer(Frame.Wet(slot))
    }

    private fun publishWet(slot: StrokeSlot, path: Path, inputMs: Long, enqueue: Boolean = true) {
        synchronized(renderLock) {
            slot.version++
            slot.latest = WetSnapshot(path, inputMs, slot.version)
        }
        if (enqueue) enqueueWetIfNeeded(slot)
    }

    // Clear and detach close an entire rendering generation. Waiting for its
    // release bounds native renderer resources even if Clear is tapped repeatedly.
    private fun retireRenderer() {
        synchronized(renderLock) { rendererSession?.closed = true }
        rendererSession = null
        val previous = renderer
        renderer = null
        if (previous != null) {
            rendererRetiring = true
            previous.release(true) {
                mainHandler.post {
                    rendererRetiring = false
                    ensureRenderer()
                }
            }
        }
    }

    private fun renewGeneration() {
        retireRenderer()
        generation = Any()
        latestDry = DrySnapshot(generation, completed.toList(), null)
    }

    fun cancelActive() {
        if (handle != 0L) { NativeInk.release(handle); handle = 0L; canceledCount++ }
        val slot = activeSlot
        synchronized(renderLock) {
            slot?.let { it.canceled = true; it.latest = null }
        }
        activeSlot = null
        pointerId = -1
        activePath = Path()
        predictor = MotionEventPredictor.newInstance(this)
        // Renderer.cancel() also drops queued dry commits from earlier strokes.
        // A front-only clear preserves those immutable commit snapshots.
        if (slot != null) renderer?.renderFrontBufferedLayer(Frame.Cancel(slot))
    }

    fun clearInk() {
        cancelActive()
        completed.clear()
        started = 0
        renewGeneration()
        ensureRenderer()
    }

    override fun onDetachedFromWindow() {
        cancelActive()
        renewGeneration()
        super.onDetachedFromWindow()
    }

    private fun sample(x: Float, y: Float, time: Long, pressure: Float) {
        check(samples < 2048) { "Demo stroke reached its 2048-sample limit" }
        check(time >= startMs && time >= lastMs) { "Out-of-order input" }
        check(NativeInk.append(handle, x.toDouble(), y.toDouble(), time - startMs, pressure.coerceIn(0f, 1f).toDouble()) >= 0) { "Core rejected input" }
        lastMs = time
        samples++
        polygonCount = NativeInk.appendContours(activePath, handle, polygonCount)
    }

    private fun appendEvent(event: MotionEvent, index: Int) {
        for (history in 0 until event.historySize) {
            sample(event.getHistoricalX(index, history), event.getHistoricalY(index, history),
                event.getHistoricalEventTime(history), event.getHistoricalPressure(index, history))
        }
        sample(event.getX(index), event.getY(index), event.eventTime, event.getPressure(index))
    }

    private fun render(event: MotionEvent) {
        val display = Path(activePath)
        val predicted = predictor.predict()
        try {
            if (predicted != null && predicted.eventTime > lastMs && predicted.eventTime - lastMs <= 100) {
                val index = predicted.findPointerIndex(pointerId)
                if (index >= 0) {
                    val preview = NativeInk.preview(handle, predicted.getX(index).toDouble(), predicted.getY(index).toDouble(),
                        predicted.eventTime - startMs, predicted.getPressure(index).coerceIn(0f, 1f).toDouble())
                    if (preview != 0L) {
                        try { NativeInk.appendContours(display, preview, polygonCount) }
                        finally { NativeInk.release(preview) }
                    }
                }
            }
        } finally { predicted?.recycle() }
        publishWet(checkNotNull(activeSlot), display, event.eventTime)
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        try {
            if (event.actionMasked == MotionEvent.ACTION_CANCEL || event.pointerCount > 1 || (event.flags and 0x20) != 0) {
                cancelActive(); return true
            }
            if (event.actionMasked == MotionEvent.ACTION_DOWN) {
                cancelActive()
                check(started < 64) { "Clear the demo canvas after 64 strokes" }
                handle = NativeInk.begin(0, 12.0, 0.25)
                check(handle != 0L) { "Core could not begin a stroke" }
                activeSlot = StrokeSlot(generation)
                started++
                pointerId = event.getPointerId(event.actionIndex)
                startMs = event.eventTime; lastMs = startMs; samples = 0; polygonCount = 0
                activePath = Path(); lastFailure = null
                requestUnbufferedDispatch(event)
            }
            if (handle == 0L) return true
            val index = event.findPointerIndex(pointerId)
            if (index < 0) { cancelActive(); return true }
            predictor.record(event)
            if (event.actionMasked in setOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_MOVE, MotionEvent.ACTION_UP)) {
                appendEvent(event, index)
                metrics.record("rust", "native_append_and_path", event.eventTime)
                if (event.actionMasked == MotionEvent.ACTION_UP) {
                    lastWetHash = NativeInk.hash(handle)
                    check(NativeInk.finish(handle) == 0) { "Core could not finish stroke" }
                    lastDryHash = NativeInk.hash(handle)
                    check(lastWetHash == lastDryHash) { "Wet/dry geometry changed" }
                    val slot = checkNotNull(activeSlot)
                    val canonical = Path(activePath)
                    publishWet(slot, canonical, event.eventTime, enqueue = false)
                    completed.add(canonical); committedCount++
                    latestDry = DrySnapshot(generation, completed.toList(), event.eventTime)
                    NativeInk.release(handle); handle = 0L; pointerId = -1
                    activeSlot = null
                    // Keep the final real path in the front layer until the
                    // renderer atomically commits and clears that layer.
                    renderer?.renderFrontBufferedLayer(Frame.Commit(latestDry, slot))
                    renderer?.commit()
                    performClick()
                } else render(event)
            }
        } catch (error: IllegalArgumentException) {
            lastFailure = error.message; cancelActive()
        } catch (error: IllegalStateException) {
            lastFailure = error.message; cancelActive()
        }
        return true
    }

    override fun performClick(): Boolean { super.performClick(); return true }
}
