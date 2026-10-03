package com.visualworkbench.android.editor

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.PorterDuff
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.ViewConfiguration
import androidx.graphics.lowlatency.CanvasFrontBufferedRenderer
import androidx.graphics.surface.SurfaceControlCompat
import androidx.input.motionprediction.MotionEventPredictor
import com.visualworkbench.android.input.CanvasGestureRouter
import com.visualworkbench.android.input.PointerTool
import com.visualworkbench.android.input.StylusInput

/** One immutable scene slot per surface generation bounds queued render memory. */
internal class CanvasSurface(context: Context, val controller: EditorController) : SurfaceView(context) {
    private class RenderSession {
        var closed = false
        var latest: CanvasScene? = null
        var frontQueued = false
        var dryQueued = false
        var drawn = 0L
        var frontDrawn = 0L
    }
    private data class Frame(val session: RenderSession, val dry: Boolean, val inputMs: Long)
    private val lock = Any()
    private val main = Handler(Looper.getMainLooper())
    private val stylus = StylusInput()
    private val router = CanvasGestureRouter(controller, ViewConfiguration.get(context).scaledTouchSlop.toDouble())
    private var predictor = MotionEventPredictor.newInstance(this)
    private var renderer: CanvasFrontBufferedRenderer<Frame>? = null
    private var session: RenderSession? = null
    private var retiring = false
    private var surfaceWidth = 0
    private var surfaceHeight = 0
    private var latest: CanvasScene? = null
    private var lastInputMs = 0L
    private var attached = false
    private val longPress = Runnable { router.longPress(SystemClock.uptimeMillis()) }

    init {
        contentDescription = "Document canvas. Pen draws; one finger pans; two fingers zoom and rotate."
        isFocusable = true
        holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) { ensureRenderer() }
            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
                val changed = surfaceWidth != width || surfaceHeight != height
                surfaceWidth = width; surfaceHeight = height
                if (changed) { cancelInput(); retireRenderer(); controller.viewport(width, height, resources.displayMetrics.density) }
                ensureRenderer()
            }
            override fun surfaceDestroyed(holder: SurfaceHolder) {
                surfaceWidth = 0; surfaceHeight = 0; cancelInput(); retireRenderer()
            }
        })
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow(); attached = true
        controller.attachCanvas(::publish)
        ensureRenderer()
    }
    override fun onDetachedFromWindow() {
        attached = false; cancelInput(); controller.detachCanvas(); retireRenderer()
        super.onDetachedFromWindow()
    }

    fun cancelInput() {
        main.removeCallbacks(longPress); router.cancel()
        predictor = MotionEventPredictor.newInstance(this)
    }

    fun updatePreferences() { router.drawWithFinger = controller.preferences.drawWithFinger }

    private fun publish(scene: CanvasScene) {
        latest = scene
        val target = session ?: return
        synchronized(lock) { if (!target.closed) target.latest = scene }
        enqueue(target, !wet(scene))
    }

    private fun enqueue(target: RenderSession, dry: Boolean) {
        val current = renderer ?: return
        val send = synchronized(lock) {
            if (target.closed || target !== session) false
            else if (dry) { if (target.dryQueued) false else { target.dryQueued = true; true } }
            else { if (target.frontQueued) false else { target.frontQueued = true; true } }
        }
        if (!send) return
        val frame = Frame(target, dry, lastInputMs)
        if (dry) current.renderMultiBufferedLayer(listOf(frame)) else current.renderFrontBufferedLayer(frame)
    }

    private fun ensureRenderer() {
        if (!attached || !holder.surface.isValid || width <= 0 || height <= 0 || surfaceWidth <= 0 || surfaceHeight <= 0 || renderer != null || retiring) return
        val currentSession = RenderSession().also { it.latest = latest }
        session = currentSession
        renderer = CanvasFrontBufferedRenderer(this, object : CanvasFrontBufferedRenderer.Callback<Frame> {
            override fun onDrawFrontBufferedLayer(canvas: Canvas, bufferWidth: Int, bufferHeight: Int, param: Frame) {
                val snapshot = synchronized(lock) {
                    currentSession.frontQueued = false
                    if (currentSession.closed || param.session !== currentSession) null else currentSession.latest?.also { currentSession.frontDrawn = it.revision }
                } ?: return
                EditorDrawing.draw(canvas, snapshot)
                if (param.inputMs > 0) main.post { if (!currentSession.closed) controller.drawCallback(param.inputMs) }
            }
            override fun onDrawMultiBufferedLayer(canvas: Canvas, bufferWidth: Int, bufferHeight: Int, params: Collection<Frame>) {
                val snapshot = synchronized(lock) {
                    currentSession.dryQueued = false
                    if (currentSession.closed) null else currentSession.latest?.also { currentSession.drawn = it.revision }
                }
                if (snapshot == null) canvas.drawColor(Color.TRANSPARENT, PorterDuff.Mode.CLEAR)
                else EditorDrawing.draw(canvas, snapshot)
            }
            override fun onFrontBufferedLayerRenderComplete(frontBufferedLayerSurfaceControl: SurfaceControlCompat,
                                                            transaction: SurfaceControlCompat.Transaction) {
                synchronized(lock) {
                    // A stale wet callback cannot cover a newer saved dry frame.
                    if (currentSession.closed || currentSession.frontDrawn <= currentSession.drawn)
                        transaction.setVisibility(frontBufferedLayerSurfaceControl, false)
                }
                main.post { redrawNewer(currentSession) }
            }
            override fun onMultiBufferedLayerRenderComplete(frontBufferedLayerSurfaceControl: SurfaceControlCompat,
                                                            multiBufferedLayerSurfaceControl: SurfaceControlCompat,
                                                            transaction: SurfaceControlCompat.Transaction) {
                synchronized(lock) {
                    currentSession.frontDrawn = minOf(currentSession.frontDrawn, currentSession.drawn)
                    if (currentSession.closed) {
                        transaction.setVisibility(frontBufferedLayerSurfaceControl, false)
                        transaction.setVisibility(multiBufferedLayerSurfaceControl, false)
                    }
                }
                // renderMultiBufferedLayer hides the front at this transaction.
                // Reissue any newer wet scene afterward so a dry commit cannot
                // make a following stroke disappear until its next input event.
                main.post { redrawNewer(currentSession) }
            }
        })
        enqueue(currentSession, true)
    }

    private fun redrawNewer(target: RenderSession) {
        val state = synchronized(lock) {
            if (target.closed || target !== session) null else target.latest?.let {
                val dry = !wet(it)
                if (it.revision > (if (dry) target.drawn else maxOf(target.drawn, target.frontDrawn))) dry else null
            }
        }
        if (state != null) enqueue(target, state)
    }

    private fun wet(scene: CanvasScene) = scene.provisional.isNotEmpty() || scene.transformed.isNotEmpty() || scene.hidden.isNotEmpty()

    private fun retireRenderer() {
        synchronized(lock) { session?.closed = true }
        session = null
        val previous = renderer; renderer = null
        if (previous != null) {
            retiring = true
            previous.release(true) { main.post { retiring = false; ensureRenderer() } }
        }
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        lastInputMs = event.eventTime
        try {
            if (event.actionMasked == MotionEvent.ACTION_DOWN) {
                requestUnbufferedDispatch(event); parent?.requestDisallowInterceptTouchEvent(true)
                main.removeCallbacks(longPress); main.postDelayed(longPress, 500)
            }
            predictor.record(event)
            val packet = stylus.snapshot(event)
            router.accept(packet)
            if (event.actionMasked in setOf(MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL)) {
                main.removeCallbacks(longPress); controller.predict(null)
                predictor = MotionEventPredictor.newInstance(this)
                if (event.actionMasked == MotionEvent.ACTION_UP) performClick()
            } else {
                val prediction = predictor.predict()
                try {
                    val sample = prediction?.let { stylus.snapshot(it).frames.lastOrNull()?.pointers?.firstOrNull { point ->
                        point.tool == PointerTool.Pen || point.tool == PointerTool.Eraser || controller.preferences.drawWithFinger
                    } }
                    controller.predict(sample)
                } finally { prediction?.recycle() }
            }
        } catch (_: IllegalArgumentException) { cancelInput() }
        catch (_: IllegalStateException) { cancelInput() }
        return true
    }
    override fun onHoverEvent(event: MotionEvent): Boolean {
        try { router.accept(stylus.snapshot(event)) } catch (_: IllegalArgumentException) { controller.predict(null) }
        return true
    }
    override fun onGenericMotionEvent(event: MotionEvent): Boolean {
        if (event.isFromSource(android.view.InputDevice.SOURCE_STYLUS)) return onHoverEvent(event)
        return super.onGenericMotionEvent(event)
    }
    override fun performClick(): Boolean { super.performClick(); return true }
}
