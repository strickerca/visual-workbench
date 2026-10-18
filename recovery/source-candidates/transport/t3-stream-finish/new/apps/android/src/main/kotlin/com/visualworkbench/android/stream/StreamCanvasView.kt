package com.visualworkbench.android.stream

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import com.visualworkbench.android.editor.*
import com.visualworkbench.android.input.*
import com.visualworkbench.shared.CaptureStreamDraw
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlin.math.min

/** Main-thread draw/receipt/contact serialization. A receipt means this View's
 * software Canvas consumed these pixels, not a measured display photon time.
 * Native remains sole owner of the acknowledged frame used by firstContact. */
internal class StreamCanvasView(context: Context, private val owner: AndroidStreamController) : View(context) {
    private data class Pending(val bitmap: Bitmap, val drawn: CompletableDeferred<Boolean>, val didDraw: () -> Unit)
    private var pending: Pending? = null
    private var shown: Bitmap? = null
    private var scene: CanvasScene? = null
    private var child: EditorController? = null
    private var router: CanvasGestureRouter? = null
    private var attached = false
    private var contact = false
    private var bufferedSamples = 0
    private val packets = ArrayList<PointerPacket>()
    private val stylus = StylusInput()
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.FILTER_BITMAP_FLAG)

    init { setLayerType(LAYER_TYPE_SOFTWARE, null); contentDescription = "Live computer capture. Pen contact freezes the displayed frame for annotation." }
    override fun onAttachedToWindow() { super.onAttachedToWindow(); attached = true; owner.attachView(this) }
    override fun onDetachedFromWindow() {
        attached = false; discardContact(); settleUndrawn(); child?.detachCanvas(); child = null; router = null; scene = null
        shown?.recycle(); shown = null; owner.detachView(this); super.onDetachedFromWindow()
    }
    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        discardContact(); child?.viewport(w, h, resources.displayMetrics.density)
        if (w > 0 && h > 0) owner.viewportReady(this)
    }
    override fun onDraw(canvas: Canvas) {
        canvas.drawColor(Color.rgb(26, 28, 33))
        val snapshot = scene
        if (snapshot != null) { EditorDrawing.draw(canvas, snapshot); return }
        val next = pending
        val bitmap = next?.bitmap ?: shown ?: return
        val scale = min(width.toFloat() / bitmap.width, height.toFloat() / bitmap.height) * .92f
        val w = bitmap.width * scale; val h = bitmap.height * scale
        canvas.drawBitmap(bitmap, null, RectF((width-w)/2, (height-h)/2, (width+w)/2, (height+h)/2), paint)
        if (next != null && pending === next) {
            pending = null
            val old = shown; shown = next.bitmap
            // Actual draw and receipt completion share Main with physical DOWN.
            next.didDraw(); next.drawn.complete(true)
            if (old !== next.bitmap) old?.recycle()
        }
    }
    suspend fun show(draw: CaptureStreamDraw, didDraw: () -> Unit): Boolean {
        check(pending == null)
        if (!attached || contact || child != null || width <= 0 || height <= 0) return false
        val bitmap = readStreamBitmap(draw)
        var transferred = false
        try {
            if (!attached || contact || child != null) return false
            val next = Pending(bitmap, CompletableDeferred(), didDraw); pending = next; transferred = true; invalidate()
            try { return next.drawn.await() }
            finally { if (pending === next) { pending = null; next.drawn.complete(false); bitmap.recycle() } }
        } finally { if (!transferred) bitmap.recycle() }
    }
    fun install(editor: EditorController) {
        check(attached)
        child = editor
        editor.viewport(width, height, resources.displayMetrics.density); editor.fit()
        editor.attachCanvas { scene = it; invalidate() }
        router = CanvasGestureRouter(editor, ViewConfiguration.get(context).scaledTouchSlop.toDouble()).also { it.drawWithFinger = editor.preferences.drawWithFinger }
        // Real accepted packet history, never prediction, is replayed unchanged
        // through the same production router after the native draft opens.
        packets.forEach { router?.accept(it) }; packets.clear(); bufferedSamples = 0; contact = false
        shown?.recycle(); shown = null; invalidate()
    }
    fun retireEditor() { discardContact(); child?.detachCanvas(); child = null; router = null; scene = null; invalidate() }
    fun sealInput() { discardContact(); owner.inputSealed = true }
    fun inputIdle(): Boolean = !contact && packets.isEmpty()
    fun discardContact() { router?.cancel(); packets.clear(); bufferedSamples = 0; contact = false }
    private fun settleUndrawn() { pending?.let { pending = null; it.drawn.complete(false); it.bitmap.recycle() } }
    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (owner.inputSealed) return true
        val packet = try { stylus.snapshot(event) } catch (_: IllegalArgumentException) { discardContact(); owner.message("This input batch was too large; the contact was cancelled."); return true }
        router?.let {
            it.accept(packet)
            if (event.actionMasked == MotionEvent.ACTION_DOWN) contact = true
            if (event.actionMasked == MotionEvent.ACTION_UP || event.actionMasked == MotionEvent.ACTION_CANCEL) contact = false
            return true
        }
        var start = false
        if (!contact) {
            val pressed = packet.frames.lastOrNull()?.pointers?.firstOrNull { it.pointerId == packet.actionPointer }
            if (event.actionMasked != MotionEvent.ACTION_DOWN || pressed?.tool !in setOf(PointerTool.Pen, PointerTool.Eraser) || shown == null) return true
            contact = true
            // No queued unseen bitmap may subsequently replace the actual frame
            // under this DOWN. An already drawn receipt is joined by the owner.
            settleUndrawn()
            start = true
        }
        val count = packet.frames.sumOf { it.pointers.size }
        if (packet.canceled || packets.size >= 256 || count > 4096 - bufferedSamples) {
            discardContact(); owner.message("The first contact could not be retained. The frozen draft is preserved; draw again when it opens.")
        } else { packets.add(packet); bufferedSamples += count; if (start) owner.freeze(this) }
        return true
    }
    companion object {
        /** Two CPU bitmaps, software rendering copies and row transfer storage
         * are charged before allocation. Native stream budget is independent. */
        fun admitBitmap(width: Long, height: Long) {
            require(width in 1..50_000 && height in 1..50_000)
            require(width * 4 <= 256 * 1024)
            require(width * height <= (128L * 1024 * 1024 - 1024 * 1024) / 16) { "Live display exceeds its 128 MiB bitmap workspace." }
        }
    }
}
