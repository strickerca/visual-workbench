package com.visualworkbench.android.editor

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.PorterDuff
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
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
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** One immutable scene slot per renderer generation bounds queued render memory.
 *
 * The canvas exists only in AndroidX child layers: the SurfaceView's own opaque
 * surface is never drawn and shows black wherever no generation has a committed
 * frame. A size change is therefore make-before-break: the presented generation is
 * kept and one successor is created at the new size. The kept generation is
 * released only after the successor's first frame is committed AND the kept
 * generation's own AndroidX resize redraw (which carries SurfaceView's redraw
 * completion) has finished, or [KEPT_REDRAW_FALLBACK_MS] after the successor
 * presented. Main waits at most [SUCCESSOR_PRESENT_WAIT_MS] for the successor
 * before AndroidX resizes the kept generation in place.
 *
 * Generations: at most two (kept + successor), and no generation is allocated
 * while any release is settling. Buffer pools are bounded but not two: AndroidX
 * rebuilds the kept generation in place at the new size and releases its
 * pre-resize pools asynchronously, so up to three pool sets exist transiently
 * (successor, kept resized, kept pre-resize) and two until the kept generation
 * leaves. A further resize before the successor presents rebuilds both
 * generations in place, transiently up to four pool sets; later resizes are
 * deferred into one coalesced replacement. Giant mode keeps break-before-make so
 * its single surface reservation is never shared. */
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
        /** Released once a visible multi-buffered frame of this generation is committed. */
        val firstPresent = CountDownLatch(1)
        val presented: Boolean get() = firstPresent.count == 0L
        /** Layer of the last completed multi-buffered frame (under [lock]). */
        var lastMultiLayer: SurfaceControlCompat? = null
        /** An AndroidX in-place resize is pending: its redraw lands on a new layer (under [lock]). */
        var awaitingResizeRedraw = false
        var resizeFrom: SurfaceControlCompat? = null
    }
    private class Generation(val renderer: CanvasFrontBufferedRenderer<Frame>, val session: RenderSession)
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
    private val replacement = CanvasReplacement<Generation>()
    private val keptFallback = Runnable { replacement.fallback()?.let { release(it, CanvasRetirementReason.SurfaceChanged) } }
    private val generations = CanvasGenerations()
    private val retirementProgress=CanvasRetirementProgress()
    private val redraws = CanvasRedrawHandshake({ task, delay -> main.postDelayed(task, delay) }, { main.removeCallbacks(it) }, REDRAW_FALLBACK_MS)
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
        holder.addCallback(object : SurfaceHolder.Callback2 {
            override fun surfaceCreated(holder: SurfaceHolder) { ensureRenderer() }
            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
                val changed = surfaceWidth != width || surfaceHeight != height
                surfaceWidth = width; surfaceHeight = height
                if (changed) { cancelInput(); resize(width, height) }
                ensureRenderer()
            }
            override fun surfaceDestroyed(holder: SurfaceHolder) {
                surfaceWidth = 0; surfaceHeight = 0; cancelInput(); retireRenderer(CanvasRetirementReason.SurfaceDestroyed)
                redraws.finish()
            }
            override fun surfaceRedrawNeeded(holder: SurfaceHolder) {}
            // Finish SurfaceView's redraw only over a committed frame (bounded);
            // finishing first would cut the window hole over an empty black layer.
            // Giant frames stay hidden until their tiles are current, so giant mode
            // keeps the earlier immediate completion instead of waiting the fallback.
            override fun surfaceRedrawNeededAsync(holder: SurfaceHolder, drawingFinished: Runnable) {
                redraws.request(drawingFinished, !attached || controller.giantMode || session?.presented == true)
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
        // A retirement can complete exceptionally only after AndroidX released the
        // renderer pipeline (its document reader close failed, and that frame keeps
        // its own quarantine). Confirm either way: an unconfirmed hook gates every
        // successor canvas forever and leaves the recreated canvas black.
        retireRenderer(CanvasRetirementReason.WindowDetached).invokeOnCompletion{main.post{if(!attached)controller.canvasRendererRetired(retirementHook)}}
        redraws.finish()
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

    /** Main, for an actual SurfaceView size change. */
    private fun resize(width: Int, height: Int) {
        val density = resources.displayMetrics.density
        val current = renderer; val currentSession = session
        when (replacement.resizeAction(current != null && currentSession != null, controller.giantMode, generations.releasing)) {
            CanvasReplacement.ResizeAction.RetireAll -> {
                retireRenderer(CanvasRetirementReason.SurfaceChanged); controller.viewport(width, height, density)
            }
            CanvasReplacement.ResizeAction.Defer -> {
                // A replacement is still settling. Presented generations resize in
                // place through their own AndroidX holder callbacks (AndroidX's own
                // layer swap can leave one render's gap); one coalesced replacement
                // follows once the release settles, because an in-place rebuild can
                // lose a completion and wedge that generation's dry-frame accounting.
                currentSession?.let(::awaitResizeRedraw)
                replacement.kept?.let {
                    awaitResizeRedraw(it.session)
                    // The kept generation redraws again: restart its bounded fallback.
                    main.removeCallbacks(keptFallback); main.postDelayed(keptFallback, KEPT_REDRAW_FALLBACK_MS)
                }
                controller.viewport(width, height, density); keptFollowsLatest()
            }
            CanvasReplacement.ResizeAction.Replace -> {
                val keptGeneration = Generation(checkNotNull(current), checkNotNull(currentSession))
                // AndroidX resizes the kept generation in place right after this
                // callback; its redraw must finish before that generation may leave.
                awaitResizeRedraw(keptGeneration.session)
                replacement.keep(keptGeneration, awaitRedraw = true)
                renderer = null; session = null
                controller.viewport(width, height, density) // Its publish creates the successor from the new viewport.
                ensureRenderer()
                val successor = session ?: run { restoreKept(); return }
                keptFollowsLatest()
                // Wait (bounded) so the kept generation's AndroidX layer swap finds
                // the successor's frame already committed underneath.
                val started = SystemClock.uptimeMillis()
                val presented = successor.firstPresent.await(SUCCESSOR_PRESENT_WAIT_MS, TimeUnit.MILLISECONDS)
                Log.i(LOG_TAG, "resize successor_wait_ms=${SystemClock.uptimeMillis() - started} presented=$presented")
            }
        }
    }

    /** Main: [target]'s pending AndroidX in-place resize redraws onto a new layer. */
    private fun awaitResizeRedraw(target: RenderSession) {
        synchronized(lock) { target.awaitingResizeRedraw = true; target.resizeFrom = target.lastMultiLayer }
    }

    /** Main: [target]'s in-place resize redraw completed (its SurfaceView redraw completion ran or is running). */
    private fun resizeRedrawn(target: RenderSession) {
        val keptGeneration = replacement.kept ?: return
        if (keptGeneration.session !== target) return
        replacement.keptRedrawn(keptGeneration)?.let(::retireKeptGeneration)
    }

    /** The kept generation's own AndroidX resize redraw paints the newest scene of the same document frame. */
    private fun keptFollowsLatest() {
        val target = replacement.kept ?: return
        val scene = latest ?: return
        synchronized(lock) {
            if (!target.session.closed && target.session.latest?.documentFrame === scene.documentFrame) target.session.latest = scene
        }
    }

    /** No successor could be allocated: the kept generation stays current. */
    private fun restoreKept() {
        val target = replacement.restore() ?: return
        main.removeCallbacks(keptFallback)
        renderer = target.renderer; session = target.session
        val scene = latest ?: return
        // Same rule as publish: a different document frame needs a renderer with its reader.
        if (target.session.latest?.documentFrame !== scene.documentFrame) { retireRenderer(CanvasRetirementReason.DocumentFrameChanged); return }
        synchronized(lock) { if (!target.session.closed) target.session.latest = scene }
    }

    private fun replaceAfterSettled() {
        if (!replacement.replaceDue(generations.releasing)) return
        val current = renderer ?: return; val currentSession = session ?: return
        if (controller.giantMode) return
        // No layer swap is pending: the successor is created above the presented
        // generation and covers it once its first frame commits. A redraw still
        // pending from an earlier in-place resize must finish before it leaves.
        replacement.keep(Generation(current, currentSession), awaitRedraw = synchronized(lock) { currentSession.awaitingResizeRedraw })
        renderer = null; session = null
        ensureRenderer()
        if (session == null) restoreKept()
    }

    private fun retireKeptGeneration(target: Generation) {
        main.removeCallbacks(keptFallback)
        release(target, CanvasRetirementReason.SurfaceChanged)
    }

    private fun ensureRenderer() {
        if (!attached || !controller.canCreateCanvas(retirementHook) || !holder.surface.isValid || width <= 0 || height <= 0 || surfaceWidth <= 0 || surfaceHeight <= 0 || latest == null || !generations.canCreate(renderer != null)) return
        val currentSession = RenderSession().also { it.latest = latest;it.documentReader=latest?.documentFrame?.acquire() }
        session = currentSession
        renderer = try { CanvasFrontBufferedRenderer(this, object : CanvasFrontBufferedRenderer.Callback<Frame> {
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
                var redrawnAfterResize = false
                val visible = synchronized(lock) {
                    if (!currentSession.closed) currentSession.dryFrames.drawCompleted()
                    currentSession.frontDrawn = minOf(currentSession.frontDrawn, currentSession.drawn)
                    // Callbacks for the pre-resize layer can only precede AndroidX's
                    // release of that layer, which precedes the resize redraw on this
                    // thread: the first callback on another layer is that redraw.
                    if (currentSession.awaitingResizeRedraw &&
                        (currentSession.resizeFrom == null || multiBufferedLayerSurfaceControl !== currentSession.resizeFrom)) {
                        currentSession.awaitingResizeRedraw = false; redrawnAfterResize = true
                    }
                    currentSession.lastMultiLayer = multiBufferedLayerSurfaceControl
                    val hidden = currentSession.closed || currentSession.giantDrawn?.let{!controller.giantFrameCurrent(it)}==true
                    if (hidden) {
                        transaction.setVisibility(frontBufferedLayerSurfaceControl, false)
                        transaction.setVisibility(multiBufferedLayerSurfaceControl, false)
                    }
                    !hidden
                }
                // AndroidX runs SurfaceView's redraw completion right after this
                // callback on this thread; a release posted from Main now queues
                // behind it, so quitting this thread can no longer drop it.
                if (redrawnAfterResize) main.post { resizeRedrawn(currentSession) }
                // Presentation means committed, not recorded: only then may a kept
                // predecessor leave or SurfaceView cut its window hole over this layer.
                if (visible && !currentSession.presented) {
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S)
                        transaction.addTransactionCommittedListener({ it.run() }, object : SurfaceControlCompat.TransactionCommittedListener {
                            override fun onTransactionCommitted() { presented(currentSession) }
                        })
                    // No committed listener before API 31; Main may be waiting in resize.
                    else presented(currentSession)
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
        }) } catch (error: Throwable) {
            // A renderer that failed to construct owns nothing; never leak the reader lease.
            synchronized(lock) { currentSession.closed = true; currentSession.dryFrames.close() }
            if (session === currentSession) session = null
            currentSession.documentReader?.close(); currentSession.documentReader = null
            throw error
        }
        enqueue(currentSession, true)
    }

    /** Any thread: the generation's first visible frame has been committed. */
    private fun presented(target: RenderSession) {
        if (target.presented) return
        target.firstPresent.countDown()
        main.post {
            if (target !== session || target.closed) return@post
            val retire = replacement.successorPresented()
            if (retire != null) retireKeptGeneration(retire)
            else if (replacement.kept != null) { main.removeCallbacks(keptFallback); main.postDelayed(keptFallback, KEPT_REDRAW_FALLBACK_MS) }
            redraws.finish()
        }
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
    /** Retires every generation, kept and current; completes after all releases settle. */
    private fun retireRenderer(reason:CanvasRetirementReason):CompletableDeferred<Unit> {
        controller.canvasRendererRetiring()
        replacement.clearDeferred()
        main.removeCallbacks(keptFallback)
        replacement.take()?.let{release(it,reason)}
        val target=session;val previous=renderer
        session=null;renderer=null
        if(previous!=null&&target!=null)release(Generation(previous,target),reason)
        else if(target!=null)synchronized(lock){target.closed=true;target.dryFrames.close()}
        return generations.joined()
    }
    private fun release(target:Generation,reason:CanvasRetirementReason) {
        val owned=target.session
        synchronized(lock) { owned.closed = true; owned.dryFrames.close() }
        val joined=generations.beginRelease()
        val release=retirementProgress.begin(reason)
        try {
            target.renderer.release(true){
                release.reached(CanvasRetirementMilestone.LibraryCallbackEntered)
                val posted=main.post{
                    release.reached(CanvasRetirementMilestone.MainCallbackEntered)
                    var readerClosed=false
                    try{
                        release.reached(CanvasRetirementMilestone.ReaderCloseRunning)
                        owned.documentReader?.close();owned.documentReader=null
                        readerClosed=true
                        release.reached(CanvasRetirementMilestone.ReaderClosed)
                        joined.complete(Unit)
                        release.reached(CanvasRetirementMilestone.CompletionPublished)
                    }
                    catch(error:Exception){
                        release.failed(if(readerClosed)CanvasRetirementMilestone.CompletionPublished else CanvasRetirementMilestone.ReaderCloseRunning)
                        joined.completeExceptionally(error)
                    }
                    finally{generations.settled(joined);ensureRenderer();replaceAfterSettled()}
                }
                if(!posted)release.failed(CanvasRetirementMilestone.MainCallbackEntered)
            }
        }catch(error:Throwable){release.failed(CanvasRetirementMilestone.ReleaseRequested);throw error}
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

    private companion object {
        /** Main-thread bound for a successor's first committed frame on resize. */
        const val SUCCESSOR_PRESENT_WAIT_MS = 150L
        /** Upper bound for holding SurfaceView's redraw completion. */
        const val REDRAW_FALLBACK_MS = 750L
        /** After the successor presented, retire the kept generation even if its resize redraw never completes. */
        const val KEPT_REDRAW_FALLBACK_MS = 500L
        const val LOG_TAG = "VW_CANVAS"
    }
}
