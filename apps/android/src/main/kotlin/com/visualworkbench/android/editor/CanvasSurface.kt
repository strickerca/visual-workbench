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
import android.view.View
import android.view.ViewConfiguration
import androidx.graphics.lowlatency.CanvasFrontBufferedRenderer
import androidx.graphics.surface.SurfaceControlCompat
import androidx.input.motionprediction.MotionEventPredictor
import com.visualworkbench.android.input.CanvasGestureRouter
import com.visualworkbench.android.input.PointerTool
import com.visualworkbench.android.input.StylusInput
import com.visualworkbench.android.documents.PreparedDocumentTile
import com.visualworkbench.android.precision.*
import com.visualworkbench.shared.GiantVisibleFrame
import android.graphics.Bitmap
import com.visualworkbench.shared.DocumentDrawLease
import kotlinx.coroutines.CompletableDeferred

/** One immutable scene slot per surface generation bounds queued render memory. */
internal class CanvasSurface(context: Context, val controller: EditorController) : SurfaceView(context) {
    private class RenderSession {
        var closed = false
        var latest: CanvasScene? = null
        var frontQueued = false
        val dryFrames = CanvasDryFrames()
        var drawn = 0L
        var frontDrawn = 0L
        var documentReader:DocumentDrawLease<PreparedDocumentTile>?=null
        var giantDrawn:GiantVisibleFrame<Bitmap>?=null
        var giantFrontDrawn:GiantVisibleFrame<Bitmap>?=null
    }
    private data class Frame(val session: RenderSession, val dry: Boolean, val inputMs: Long,
                             val dryRequest: CanvasDryFrames.Request? = null)
    private val lock = Any()
    private val main = Handler(Looper.getMainLooper())
    private val stylus = StylusInput()
    private val buttons=PrecisionButtons(controller.precision,controller::precisionInputAllowed)
    private val router = CanvasGestureRouter(controller, ViewConfiguration.get(context).scaledTouchSlop.toDouble())
    private var predictor = MotionEventPredictor.newInstance(this)
    private val retirementHook:suspend()->Unit = {retireForProject()}
    private var renderer: CanvasFrontBufferedRenderer<Frame>? = null
    private var session: RenderSession? = null
    private var retiring = false
    private var retirement=CompletableDeferred<Unit>().apply{complete(Unit)}
    private val retirementProgress=CanvasRetirementProgress()
    private var surfaceWidth = 0
    private var surfaceHeight = 0
    private var latest: CanvasScene? = null
    private var lastInputMs = 0L
    private var attached = false
    private val longPress = Runnable { router.longPress(SystemClock.uptimeMillis()) }

    init {
        contentDescription = "Document canvas. Pen draws; one finger pans; two fingers zoom and rotate."
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
        isFocusable = true
        holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) { ensureRenderer() }
            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
                val changed = surfaceWidth != width || surfaceHeight != height
                surfaceWidth = width; surfaceHeight = height
                if (changed) { cancelInput(); retireRenderer(CanvasRetirementReason.SurfaceChanged); controller.viewport(width, height, resources.displayMetrics.density) }
                ensureRenderer()
            }
            override fun surfaceDestroyed(holder: SurfaceHolder) {
                surfaceWidth = 0; surfaceHeight = 0; cancelInput(); retireRenderer(CanvasRetirementReason.SurfaceDestroyed)
            }
        })
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow(); attached = true
        controller.attachCanvas(::publish,retirementHook,::ensureRenderer,{retirementProgress.current})
        ensureRenderer()
    }
    override fun onDetachedFromWindow() {
        attached = false; cancelInput(); controller.detachCanvas(retirementHook)
        retireRenderer(CanvasRetirementReason.WindowDetached).invokeOnCompletion{error->if(error==null)main.post{if(!attached)controller.canvasRendererRetired(retirementHook)}}
        super.onDetachedFromWindow()
    }

    fun cancelInput() {
        controller.interruptAirInput(clearBarrel = true)
        main.removeCallbacks(longPress);buttons.reset();router.cancel();controller.cancelPrecision()
        predictor = MotionEventPredictor.newInstance(this)
    }

    fun updatePreferences() { router.drawWithFinger = controller.preferences.drawWithFinger }

    override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
        super.onWindowFocusChanged(hasWindowFocus)
        if (!hasWindowFocus) cancelInput()
    }

    private fun publish(scene: CanvasScene) {
        latest = scene
        if(session==null)ensureRenderer()
        val target = session ?: return
        if(target.latest?.documentFrame!==scene.documentFrame){retireRenderer(CanvasRetirementReason.DocumentFrameChanged);return}
        synchronized(lock) { if (!target.closed) target.latest = scene }
        enqueue(target, !wet(scene))
    }

    private fun enqueue(target: RenderSession, dry: Boolean) {
        val current = renderer ?: return
        val frame = synchronized(lock) {
            if (target.closed || target !== session) null
            else if (dry) target.dryFrames.request()?.let { Frame(target, true, lastInputMs, it) }
            else if (target.frontQueued) null
            else { target.frontQueued = true; Frame(target, false, lastInputMs) }
        }
        if (frame == null) return
        if (dry) current.renderMultiBufferedLayer(listOf(frame)) else current.renderFrontBufferedLayer(frame)
    }

    private fun ensureRenderer() {
        if (!attached || !controller.canCreateCanvas(retirementHook) || !holder.surface.isValid || width <= 0 || height <= 0 || surfaceWidth <= 0 || surfaceHeight <= 0 || latest == null || renderer != null || retiring) return
        val currentSession = RenderSession().also { it.latest = latest;it.documentReader=latest?.documentFrame?.acquire() }
        session = currentSession
        renderer = CanvasFrontBufferedRenderer(this, object : CanvasFrontBufferedRenderer.Callback<Frame> {
            override fun onDrawFrontBufferedLayer(canvas: Canvas, bufferWidth: Int, bufferHeight: Int, param: Frame) {
                val snapshot = synchronized(lock) {
                    currentSession.frontQueued = false
                    if (currentSession.closed || param.session !== currentSession) null else currentSession.latest?.also { currentSession.frontDrawn = it.revision;currentSession.giantFrontDrawn=it.giantFrame }
                } ?: return
                drawOwned(canvas,snapshot,currentSession)
                if (param.inputMs > 0) main.post { if (!currentSession.closed) controller.drawCallback(param.inputMs) }
            }
            override fun onDrawMultiBufferedLayer(canvas: Canvas, bufferWidth: Int, bufferHeight: Int, params: Collection<Frame>) {
                val snapshot = synchronized(lock) {
                    if (currentSession.closed) null else {
                        currentSession.dryFrames.drawStarted(params.mapNotNull { it.dryRequest })
                        currentSession.latest?.also { currentSession.drawn = it.revision;currentSession.giantDrawn=it.giantFrame }
                    }
                }
                if (snapshot == null) canvas.drawColor(Color.TRANSPARENT, PorterDuff.Mode.CLEAR)
                else drawOwned(canvas,snapshot,currentSession)
            }
            override fun onFrontBufferedLayerRenderComplete(frontBufferedLayerSurfaceControl: SurfaceControlCompat,
                                                            transaction: SurfaceControlCompat.Transaction) {
                synchronized(lock) {
                    // A stale wet callback cannot cover a newer saved dry frame.
                    if (currentSession.closed || currentSession.frontDrawn <= currentSession.drawn ||
                        currentSession.giantFrontDrawn?.let{!controller.giantFrameCurrent(it)}==true)
                        transaction.setVisibility(frontBufferedLayerSurfaceControl, false)
                }
                main.post { redrawNewer(currentSession) }
            }
            override fun onMultiBufferedLayerRenderComplete(frontBufferedLayerSurfaceControl: SurfaceControlCompat,
                                                            multiBufferedLayerSurfaceControl: SurfaceControlCompat,
                                                            transaction: SurfaceControlCompat.Transaction) {
                synchronized(lock) {
                    if (!currentSession.closed) currentSession.dryFrames.drawCompleted()
                    currentSession.frontDrawn = minOf(currentSession.frontDrawn, currentSession.drawn)
                    if (currentSession.closed || currentSession.giantDrawn?.let{!controller.giantFrameCurrent(it)}==true) {
                        transaction.setVisibility(frontBufferedLayerSurfaceControl, false)
                        transaction.setVisibility(multiBufferedLayerSurfaceControl, false)
                    }
                }
                currentSession.giantDrawn?.let{frame->main.post{
                    if(!currentSession.closed&&currentSession===session&&controller.giantFrameCurrent(frame))
                        controller.giantRenderCompleted(frame,retirementHook)
                }}
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

    private fun wet(scene: CanvasScene): Boolean {
        val composed = (scene.document.hasToneAdjustments && scene.background != null) ||
            scene.giantFrame?.content == com.visualworkbench.shared.GiantTileContent.Composed
        val baked = bakedCanonicalIds(scene.document, composed)
        return scene.provisional.isNotEmpty() || scene.transformed.isNotEmpty() ||
            transientHiddenIds(scene.hidden, baked).isNotEmpty() || scene.documentDraft != null
    }

    private fun drawOwned(canvas:Canvas,snapshot:CanvasScene,target:RenderSession) {
        if(snapshot.giantMode){
            synchronized(lock){
                val frame=snapshot.giantFrame
                if(target.closed||target!==session||target.latest!==snapshot||frame==null||!controller.giantFrameCurrent(frame)){
                    canvas.drawColor(Color.TRANSPARENT,PorterDuff.Mode.CLEAR);return
                }
                PrecisionDrawing.draw(canvas,snapshot)
            }
            return
        }
        val reader=target.documentReader
        if(snapshot.documentFrame==null)PrecisionDrawing.draw(canvas,snapshot)
        else reader?.read{tiles->PrecisionDrawing.draw(canvas,snapshot.copy(documentTiles=tiles))}
    }
    private suspend fun retireForProject(){latest=null;retireRenderer(CanvasRetirementReason.ProjectRetirement).await()}
    private fun retireRenderer(reason:CanvasRetirementReason):CompletableDeferred<Unit> {
        controller.canvasRendererRetiring()
        val target=session
        synchronized(lock) { target?.closed = true; target?.dryFrames?.close() }
        session=null
        val previous=renderer;renderer=null
        if(previous!=null){
            retiring=true;retirement=CompletableDeferred()
            val joined=retirement
            val release=retirementProgress.begin(reason)
            try {
                previous.release(true){
                    release.reached(CanvasRetirementMilestone.LibraryCallbackEntered)
                    val posted=main.post{
                        release.reached(CanvasRetirementMilestone.MainCallbackEntered)
                        var readerClosed=false
                        try{
                            release.reached(CanvasRetirementMilestone.ReaderCloseRunning)
                            target?.documentReader?.close();target?.documentReader=null
                            readerClosed=true
                            release.reached(CanvasRetirementMilestone.ReaderClosed)
                            joined.complete(Unit)
                            release.reached(CanvasRetirementMilestone.CompletionPublished)
                        }
                        catch(error:Exception){
                            release.failed(if(readerClosed)CanvasRetirementMilestone.CompletionPublished else CanvasRetirementMilestone.ReaderCloseRunning)
                            joined.completeExceptionally(error)
                        }
                        finally{retiring=false;ensureRenderer()}
                    }
                    if(!posted)release.failed(CanvasRetirementMilestone.MainCallbackEntered)
                }
            }catch(error:Throwable){release.failed(CanvasRetirementMilestone.ReleaseRequested);throw error}
        }
        return retirement
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
            controller.observeAirBarrel(packet)
            if(event.actionMasked!=MotionEvent.ACTION_DOWN)buttons.accept(packet)
            router.accept(packet)
            if(event.actionMasked==MotionEvent.ACTION_DOWN)buttons.accept(packet)
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
        try { val packet=stylus.snapshot(event);controller.observeAirBarrel(packet);buttons.accept(packet);router.accept(packet) }
        catch (_: IllegalArgumentException) { controller.interruptAirInput();controller.predict(null) }
        return true
    }
    override fun onGenericMotionEvent(event: MotionEvent): Boolean {
        if (event.isFromSource(android.view.InputDevice.SOURCE_STYLUS)) return onHoverEvent(event)
        return super.onGenericMotionEvent(event)
    }
    override fun performClick(): Boolean { super.performClick(); return true }
}
