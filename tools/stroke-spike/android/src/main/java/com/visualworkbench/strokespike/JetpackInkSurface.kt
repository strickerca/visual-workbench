package com.visualworkbench.strokespike

import android.content.Context
import android.graphics.Color
import android.graphics.Canvas
import android.graphics.Matrix
import android.view.MotionEvent
import android.widget.FrameLayout
import androidx.ink.authoring.InProgressStrokeId
import androidx.ink.authoring.InProgressStrokesFinishedListener
import androidx.ink.authoring.InProgressStrokesView
import androidx.ink.brush.Brush
import androidx.ink.brush.StockBrushes
import androidx.ink.strokes.Stroke
import androidx.input.motionprediction.MotionEventPredictor

internal class JetpackInkSurface(context: Context, private val metrics: StrokeMetrics) : FrameLayout(context) {
    private val ink = InProgressStrokesView(context)
    private var predictor = MotionEventPredictor.newInstance(ink)
    private val brush = Brush.createWithColorIntArgb(StockBrushes.pressurePen(), Color.rgb(24, 98, 121), 12f, 0.1f)
    private var active: InProgressStrokeId? = null
    private var pointerId = -1
    private var samples = 0
    private var started = 0
    internal var readyForInput = false
        private set
    private val completedTimes = mutableMapOf<InProgressStrokeId, Long>()
    private val clearedPending = mutableSetOf<InProgressStrokeId>()
    internal val visibleFinishedCount: Int get() = ink.getFinishedStrokes().size
    internal val pendingFinishedCount: Int get() = completedTimes.size + clearedPending.size
    var committedCount = 0
        private set
    var canceledCount = 0
        private set
    var lastFailure: String? = null
        private set

    init {
        setBackgroundColor(Color.WHITE)
        addView(ink, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        ink.addFinishedStrokesListener(object : InProgressStrokesFinishedListener {
            override fun onStrokesFinished(strokes: Map<InProgressStrokeId, Stroke>) {
                for (id in strokes.keys) {
                    if (clearedPending.remove(id)) {
                        ink.removeFinishedStrokes(setOf(id))
                    } else {
                        completedTimes.remove(id)?.let { metrics.record("jetpack", "finished_stroke_callback", it) }
                        committedCount++
                    }
                }
            }
        })
        // Create the Android 11 front-buffer surface before the first pointer.
        // Starting and finishing inside the attachment/layout transaction can
        // strand that backend's initial render request before a surface exists.
        ink.eagerInit()
        // The parent owns input so tests and physical input take the same path.
        isClickable = true
    }

    override fun onInterceptTouchEvent(event: MotionEvent): Boolean = true
    fun cancelActive() {
        active?.let { ink.cancelStroke(it, null); completedTimes.remove(it); canceledCount++ }
        active = null; pointerId = -1
        predictor = MotionEventPredictor.newInstance(ink)
    }
    fun clearInk() {
        cancelActive()
        clearedPending.addAll(completedTimes.keys)
        completedTimes.clear()
        ink.removeFinishedStrokes(ink.getFinishedStrokes().keys.toSet())
        started = 0
    }
    override fun dispatchDraw(canvas: Canvas) {
        super.dispatchDraw(canvas)
        readyForInput = isAttachedToWindow && width > 0 && height > 0 && ink.width > 0 && ink.height > 0
    }
    override fun onDetachedFromWindow() { readyForInput = false; cancelActive(); super.onDetachedFromWindow() }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (!readyForInput) return false
        try {
            if (event.actionMasked == MotionEvent.ACTION_CANCEL || event.pointerCount > 1 || (event.flags and 0x20) != 0) {
                cancelActive(); return true
            }
            predictor.record(event)
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    cancelActive()
                    check(started < 64 && completedTimes.size + clearedPending.size < 64) { "Clear the canvas or wait for pending strokes" }
                    pointerId = event.getPointerId(event.actionIndex)
                    ink.requestUnbufferedDispatch(event)
                    predictor.record(event)
                    active = ink.startStroke(event, pointerId, brush, Matrix(), Matrix())
                    samples = 1; started++; lastFailure = null
                }
                MotionEvent.ACTION_MOVE -> active?.let {
                    samples += event.historySize + 1
                    check(samples <= 2048) { "Demo stroke reached its 2048-sample limit" }
                    val prediction = predictor.predict()
                    try { ink.addToStroke(event, pointerId, it, prediction) }
                    finally { prediction?.recycle() }
                }
                MotionEvent.ACTION_UP -> active?.let {
                    samples += event.historySize + 1
                    check(samples <= 2048) { "Demo stroke reached its 2048-sample limit" }
                    completedTimes[it] = event.eventTime
                    ink.finishStroke(event, pointerId, it)
                    active = null; pointerId = -1
                    performClick()
                }
            }
            metrics.record("jetpack", "authoring_input_callback", event.eventTime)
        } catch (error: IllegalArgumentException) {
            lastFailure = error.message; cancelActive()
        } catch (error: IllegalStateException) {
            lastFailure = error.message; cancelActive()
        }
        return true
    }
    override fun performClick(): Boolean { super.performClick(); return true }
}
