package com.visualworkbench.penprobe

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.PorterDuff
import android.os.Build
import android.view.Choreographer
import android.view.MotionEvent
import android.view.SurfaceView
import androidx.graphics.lowlatency.CanvasFrontBufferedRenderer

/** Diagnostic dots only; product stroke geometry and palm rejection belong to later tasks. */
class ProbeSurface(context: Context, private val recorder: TraceRecorder, private val changed: () -> Unit) : SurfaceView(context) {
    data class Dot(val x: Float, val y: Float, val pressure: Float, val inputNs: Long, val generation: Long)
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.rgb(24, 98, 121) }
    private var renderer: CanvasFrontBufferedRenderer<Dot>? = null
    private var framePending = false
    private var latestInputNs = 0L
    private var latestGeneration = 0L
    private val frameCallback = Choreographer.FrameCallback { frameTime ->
        framePending = false
        recorder.timing(latestGeneration, "choreographer_callback", latestInputNs, System.nanoTime(), frameTime)
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        renderer = CanvasFrontBufferedRenderer(this, object : CanvasFrontBufferedRenderer.Callback<Dot> {
            override fun onDrawFrontBufferedLayer(canvas: Canvas, bufferWidth: Int, bufferHeight: Int, param: Dot) {
                canvas.drawCircle(param.x, param.y, 2f + 5f * param.pressure.coerceIn(0f, 1f), paint)
                recorder.timing(param.generation, "front_buffer_draw_callback", param.inputNs, System.nanoTime())
            }
            override fun onDrawMultiBufferedLayer(canvas: Canvas, bufferWidth: Int, bufferHeight: Int, params: Collection<Dot>) {
                canvas.drawColor(Color.WHITE, PorterDuff.Mode.SRC)
                for (dot in params) canvas.drawCircle(dot.x, dot.y, 2f + 5f * dot.pressure.coerceIn(0f, 1f), paint)
            }
        })
    }

    fun clearInk() { renderer?.clear() }

    override fun onDetachedFromWindow() {
        Choreographer.getInstance().removeFrameCallback(frameCallback)
        framePending = false
        renderer?.release(true) {}; renderer = null
        super.onDetachedFromWindow()
    }

    private fun record(event: MotionEvent): Boolean {
        val captured = recorder.motion(event)
        val inputNs = if (Build.VERSION.SDK_INT >= 34) event.eventTimeNanos else event.eventTime * 1_000_000L
        if (event.actionMasked in setOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_MOVE, MotionEvent.ACTION_UP)) {
            renderer?.renderFrontBufferedLayer(Dot(event.x, event.y, event.pressure, inputNs, recorder.generation))
            if (event.actionMasked == MotionEvent.ACTION_UP) renderer?.commit()
        }
        if (captured) {
            latestInputNs = inputNs
            latestGeneration = recorder.generation
            if (!framePending) {
                framePending = true
                Choreographer.getInstance().postFrameCallback(frameCallback)
            }
        }
        if (captured || recorder.atLimit()) changed()
        return true
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_DOWN) requestUnbufferedDispatch(event)
        record(event)
        if (event.actionMasked == MotionEvent.ACTION_UP) performClick()
        return true
    }
    override fun onHoverEvent(event: MotionEvent): Boolean = record(event)
    override fun onGenericMotionEvent(event: MotionEvent): Boolean = record(event)
    override fun performClick(): Boolean { super.performClick(); return true }
}
