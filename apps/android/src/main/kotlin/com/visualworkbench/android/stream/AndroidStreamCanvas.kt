package com.visualworkbench.android.stream

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.ViewConfiguration
import com.visualworkbench.android.editor.CanvasScene
import com.visualworkbench.android.editor.EditorDrawing
import com.visualworkbench.android.input.*
import com.visualworkbench.shared.CaptureStreamFrame
import com.visualworkbench.shared.Camera
import com.visualworkbench.shared.Point
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow

internal const val STREAM_ANDROID_SURFACE_BYTES = 100_663_296uL

/** Software Surface Canvas: positive completion follows draw AND successful
 * unlockCanvasAndPost, not decoding, invalidation or Compose recomposition.
 * All access is on Main under the lifecycle draw/freeze gate. */
internal class AndroidStreamCanvas(
    context: Context,
    input: CanvasGestureSink,
    private val admitted: () -> Boolean,
    private val changed: () -> Unit,
    private val viewport: (Int, Int, Float) -> Unit,
    private val interrupted: () -> Unit,
    private val resumed: () -> Unit,
) : SurfaceView(context) {
    private val stylus = StylusInput()
    private val router = CanvasGestureRouter(input, ViewConfiguration.get(context).scaledTouchSlop.toDouble())
    private val main = Handler(Looper.getMainLooper())
    private val longPress = Runnable { router.longPress(SystemClock.uptimeMillis()) }
    private var active = false
    private var retiring = false
    private var requestedWidth = 1
    private var requestedHeight = 1
    private var actualWidth = 0
    private var actualHeight = 0
    private var destroyed = CompletableDeferred<Unit>().apply { complete(Unit) }
    private val mutableIssue = MutableStateFlow<String?>(null)
    val issue = mutableIssue.asStateFlow()
    var scene: CanvasScene? = null

    init {
        check(admitted())
        contentDescription = "Live capture canvas. Pen freezes the displayed frame for annotation."
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
        holder.setFixedSize(1, 1) // No unadmitted window-sized buffer before sizing.
        holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) {
                check(admitted())
                active = true; destroyed = CompletableDeferred(); resumed(); changed()
            }
            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
                actualWidth = width; actualHeight = height; changed()
            }
            override fun surfaceDestroyed(holder: SurfaceHolder) {
                active = false; actualWidth = 0; actualHeight = 0
                cancelInput(); interrupted(); destroyed.complete(Unit)
            }
        })
    }

    override fun onSizeChanged(width: Int, height: Int, oldWidth: Int, oldHeight: Int) {
        super.onSizeChanged(width, height, oldWidth, oldHeight)
        cancelInput()
        if (retiring || width !in 1..8192 || height !in 1..8192 || width.toLong() * height > 5L * 1024L * 1024L) {
            if (width > 0 && height > 0 && !retiring) mutableIssue.value = "The capture surface exceeds its current display memory budget. Resize or close this view."
            requestedWidth = 1; requestedHeight = 1; holder.setFixedSize(1, 1); changed(); return
        }
        mutableIssue.value = null
        check(admitted())
        requestedWidth = width; requestedHeight = height
        holder.setFixedSize(width, height)
        viewport(width, height, resources.displayMetrics.density)
        changed()
    }

    fun drawWithFinger(value: Boolean) { router.drawWithFinger = value }
    fun cancelInput() { main.removeCallbacks(longPress); router.cancel() }
    fun liveCamera(frame: CaptureStreamFrame): Camera = Camera(
        Point(frame.width.toDouble() / 2.0, frame.height.toDouble() / 2.0),
        minOf(requestedWidth / frame.width.toDouble(), requestedHeight / frame.height.toDouble()).coerceIn(.001, 128.0),
        0.0, requestedWidth.toDouble(), requestedHeight.toDouble())

    suspend fun present(bitmap: Bitmap, frame: CaptureStreamFrame, frozen: Boolean): Boolean = withContext(Dispatchers.Main.immediate) {
        check(admitted())
        if (retiring || !active || !holder.surface.isValid || requestedWidth <= 1 || requestedHeight <= 1 ||
            actualWidth != requestedWidth || actualHeight != requestedHeight) return@withContext false
        check(!bitmap.isRecycled && bitmap.width.toUInt() == frame.width && bitmap.height.toUInt() == frame.height)
        val snapshot = if (frozen) scene else null
        if (frozen && snapshot == null) return@withContext false
        if (snapshot != null) {
            require(snapshot.document.width == frame.width && snapshot.document.height == frame.height &&
                snapshot.background == null && !snapshot.giantMode && snapshot.documentFrame == null && !snapshot.document.hasToneAdjustments &&
                snapshot.aiResults.isEmpty() && snapshot.resultDisplayReady)
            require((snapshot.objects + snapshot.provisional).all { streamAnnotationShapeSupported(it.item.shape) })
        }
        val canvas = holder.lockCanvas() ?: return@withContext false
        try {
            if (snapshot == null) {
                canvas.drawColor(Color.BLACK)
                val scale = liveCamera(frame).scale.toFloat()
                val w = bitmap.width * scale; val h = bitmap.height * scale
                val left = (canvas.width - w) / 2f; val top = (canvas.height - h) / 2f
                canvas.drawBitmap(bitmap, null, RectF(left, top, left + w, top + h), Paint())
            } else {
                // Only the background is replaced. Current camera, provisional
                // objects, transforms, selection and gesture feedback survive.
                EditorDrawing.draw(canvas, snapshot.copy(background = bitmap))
            }
        } finally { holder.unlockCanvasAndPost(canvas) }
        true
    }

    suspend fun clear(): Unit = withContext(Dispatchers.Main.immediate) {
        scene = null; cancelInput()
        if (active && holder.surface.isValid && !retiring) {
            val canvas = holder.lockCanvas()
            if (canvas != null) try { canvas.drawColor(Color.BLACK) } finally { holder.unlockCanvasAndPost(canvas) }
        }
    }

    suspend fun retire(): Unit = withContext(NonCancellable + Dispatchers.Main.immediate) {
        retiring = true; scene = null; cancelInput()
        val settled = destroyed
        visibility = View.GONE
        if (active) settled.await() // Actual Surface callback, not a timer or reservation release.
    }

    override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
        super.onWindowFocusChanged(hasWindowFocus)
        if (!hasWindowFocus) { cancelInput(); interrupted() } else if (!retiring) resumed()
    }
    override fun onDetachedFromWindow() {
        cancelInput(); interrupted(); super.onDetachedFromWindow()
    }
    override fun onTouchEvent(event: MotionEvent): Boolean {
        try {
            if (event.actionMasked == MotionEvent.ACTION_DOWN) {
                requestUnbufferedDispatch(event); parent?.requestDisallowInterceptTouchEvent(true)
                main.removeCallbacks(longPress); main.postDelayed(longPress, 500)
            }
            router.accept(stylus.snapshot(event))
            if (event.actionMasked in setOf(MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL)) {
                main.removeCallbacks(longPress)
                if (event.actionMasked == MotionEvent.ACTION_UP) performClick()
            }
        } catch (_: IllegalArgumentException) { cancelInput() }
        catch (_: IllegalStateException) { cancelInput() }
        return true
    }
    override fun onHoverEvent(event: MotionEvent): Boolean {
        try { router.accept(stylus.snapshot(event)) } catch (_: IllegalArgumentException) { cancelInput() }
        return true
    }
    override fun performClick(): Boolean { super.performClick(); return true }
}
