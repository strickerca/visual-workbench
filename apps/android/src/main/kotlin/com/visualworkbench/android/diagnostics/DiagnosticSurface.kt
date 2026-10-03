package com.visualworkbench.android.diagnostics

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.os.Build
import android.os.SystemClock
import android.view.MotionEvent
import android.view.View
import java.util.ArrayDeque

/** A bounded diagnostic trace surface; callback time is never called display latency. */
internal class DiagnosticSurface(context: Context, private val diagnostics: DiagnosticsController) : View(context) {
    private data class Dot(val x: Float, val y: Float, val width: Float)
    private val dots = ArrayDeque<Dot>()
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.rgb(24, 98, 121) }
    private var inputNs = 0L
    private var generation = 0L
    init { contentDescription = "Private pen diagnostic canvas"; isClickable = true; setBackgroundColor(Color.WHITE) }
    fun clear() { dots.clear(); invalidate() }
    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        dots.forEach { canvas.drawCircle(it.x, it.y, it.width, paint) }
        if (inputNs > 0) diagnostics.recorder.timing(generation, "view_draw_callback", inputNs, SystemClock.uptimeMillis() * 1_000_000L)
    }
    private fun record(event: MotionEvent): Boolean {
        diagnostics.event(event)
        inputNs = if (Build.VERSION.SDK_INT >= 34) event.eventTimeNanos else event.eventTime * 1_000_000L
        generation = diagnostics.recorder.generation
        if (event.actionMasked in setOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_MOVE, MotionEvent.ACTION_UP)) {
            for (history in 0..event.historySize) {
                val historical = history < event.historySize
                val x = if (historical) event.getHistoricalX(0, history) else event.x
                val y = if (historical) event.getHistoricalY(0, history) else event.y
                val pressure = if (historical) event.getHistoricalPressure(0, history) else event.pressure
                if (x.isFinite() && y.isFinite() && pressure.isFinite()) {
                    if (dots.size == 4096) dots.removeFirst()
                    dots.addLast(Dot(x, y, 2f + 5 * pressure.coerceIn(0f, 1f)))
                }
            }
            invalidate()
        }
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
