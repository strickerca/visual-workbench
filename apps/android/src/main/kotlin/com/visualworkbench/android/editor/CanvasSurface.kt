package com.visualworkbench.android.editor

import android.content.ComponentCallbacks2
import android.content.Context
import android.content.res.Configuration
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
import com.visualworkbench.android.diagnostics.PenTiming
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
 * its single surface reservation is never shared.
 *
 * A generation always starts from the scene for the surface's current size: from
 * the moment the surface has a new size until the controller has been told it,
 * every scene published on the way (cancelling the input in hand, cancelling the
 * tool) is only remembered, and the generation is created afterwards. This holds
 * for the first size of a new or returning surface as much as for a later change,
 * so a surface starts with one generation, not one for each size.
 *
 * Lifecycle diagnostics ([CanvasLife]) are recorded only when switched on, and a
 * surface reads that switch once, when it is constructed. They only observe: each
 * event costs the thread that sees it one clock read and a write of a few numbers
 * into a preallocated ring, and nothing is added to the window, the surface or a
 * frame's commit for them. Ink frames are only counted. Saved frames and their
 * requests, which hover, pan and zoom produce per input sample, are recorded in
 * full only for a few frames after each lifecycle event and otherwise counted,
 * with at most one summary a second ([CanvasLifeBudget]). */
internal class CanvasSurface(context: Context, val controller: EditorController) : SurfaceView(context) {
    private class RenderSession(val number: Int) {
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
        /** Diagnostics only: the pen sample time of the last drawn ink frame, and
         * when the saved frame now being made ready began to be drawn (0: none). */
        @Volatile var frontSampleMs = 0L
        @Volatile var savedDrawStartedNs = 0L
        /** The kept picture of the saved page that this generation's frames are
         * drawn from. Made and used on the drawing thread; discarded on Main once
         * that thread has stopped. */
        @Volatile var picture: KeptPicture? = null
        /** On-device tests only (under [lock]): takes the comparison of the next saved frame. */
        var compare: ((PictureComparison?) -> Unit)? = null
        /** Under [lock]: the kept pictures were drawn at a view that has come
         * to rest since the frame on screen, which was softer; a saved frame of
         * the same scene is owed, asked for as soon as none is in flight. */
        var settledRedraw = false
        /** Lifecycle diagnostics only, under [lock] and only while switched on: saved
         * frames started and completed, which frames are recorded, and the
         * viewport size of the scene last drawn into a saved frame. */
        val createdMs = SystemClock.uptimeMillis()
        var dryStarted = 0
        var dryCompleted = 0
        val lifeBudget = CanvasLifeBudget(LIFE_DETAIL_FRAMES, LIFE_DETAIL_REQUESTS, LIFE_SUMMARY_MS)
        var drawnViewWidth = 0
        var drawnViewHeight = 0
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
    private val keptFallback = Runnable {
        val waiting = replacement.kept
        val retire = replacement.fallback()
        if (lifeOn) life(CanvasLifeEvent.KeptFallback, (waiting?.session?.number ?: 0).toLong(), flag(retire != null),
            flag(waiting?.let { synchronized(lock) { it.session.awaitingResizeRedraw } } == true))
        retire?.let { release(it, CanvasRetirementReason.SurfaceChanged) }
    }
    private val generations = CanvasGenerations()
    private val retirementProgress=CanvasRetirementProgress()
    private val redraws = CanvasRedrawHandshake({ task, delay -> main.postDelayed(task, delay) }, { main.removeCallbacks(it) }, REDRAW_FALLBACK_MS,
        { held -> if (lifeOn) life(CanvasLifeEvent.RedrawFallback, held.toLong(), (session?.number ?: 0).toLong(), flag(session?.presented == true)) })
    private var surfaceWidth = 0
    private var surfaceHeight = 0
    private var latest: CanvasScene? = null
    private var lastInputMs = 0L
    private var attached = false
    private val longPress = Runnable { router.longPress(SystemClock.uptimeMillis()) }
    /** Lifecycle diagnostics: the switch as it was when this surface was made, this
     * surface's and its generations' ordinal numbers, and what was last written
     * about a generation that could not be created or a scene nobody owned. */
    private val lifeOn = CanvasLife.enabled()
    private val surfaceNumber = if (lifeOn) CanvasLife.nextSurface() else 0
    private var generationNumbers = 0
    private var lifeWaitReason: String? = null
    private var lifeUnownedScenes = 0
    /** Main, lifecycle diagnostics: this canvas keeps the log's writer running, from
     * its attach until its retirement after detach is confirmed. */
    private var lifeOpened = false
    /** On-device tests only: what this canvas's kept pictures did. */
    internal val pictureCounts = PictureCounts()
    /** The system asks for memory: the kept pictures go and are made again when next needed. */
    private val memory = object : ComponentCallbacks2 {
        override fun onTrimMemory(level: Int) = dropPictures()
        @Deprecated("Deprecated in Java") override fun onLowMemory() = dropPictures()
        override fun onConfigurationChanged(newConfig: Configuration) {}
    }

    init {
        contentDescription = "Document canvas. Pen draws; one finger pans; two fingers zoom and rotate."
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
        isFocusable = true
        holder.addCallback(object : SurfaceHolder.Callback2 {
            override fun surfaceCreated(holder: SurfaceHolder) {
                if (lifeOn) life(CanvasLifeEvent.SurfaceCreated, this@CanvasSurface.width.toLong(), this@CanvasSurface.height.toLong())
                ensureRenderer()
            }
            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
                val changed = surfaceWidth != width || surfaceHeight != height
                if (lifeOn) { life(CanvasLifeEvent.SurfaceChanged, width.toLong(), height.toLong(), flag(changed)); lifeDetailAgain() }
                surfaceWidth = width; surfaceHeight = height
                if (changed) {
                    // Cancelling the input in hand publishes too, and the controller
                    // has not been told this size yet: after a recreation, or when the
                    // surface comes back, it still holds the camera for the size before.
                    // No generation may start from that scene (it would be kept, and a
                    // second one built for the new size), so the cancel is held as well.
                    generations.whileViewportChanges { cancelInput() }
                    resize(width, height)
                }
                ensureRenderer()
            }
            override fun surfaceDestroyed(holder: SurfaceHolder) {
                if (lifeOn) life(CanvasLifeEvent.SurfaceDestroyed)
                surfaceWidth = 0; surfaceHeight = 0; cancelInput(); retireRenderer(CanvasRetirementReason.SurfaceDestroyed)
                finishRedraws("surface_destroyed")
            }
            override fun surfaceRedrawNeeded(holder: SurfaceHolder) {}
            // Finish SurfaceView's redraw only over a committed frame (bounded);
            // finishing first would cut the window hole over an empty black layer.
            // Giant frames stay hidden until their tiles are current, so giant mode
            // keeps the earlier immediate completion instead of waiting the fallback.
            override fun surfaceRedrawNeededAsync(holder: SurfaceHolder, drawingFinished: Runnable) {
                val presented = !attached || controller.giantMode || session?.presented == true
                // Every generation alive now also holds this redraw until its own next frame.
                if (lifeOn) {
                    life(CanvasLifeEvent.RedrawRequest, flag(presented), redraws.waiting.toLong(), lifeCurrent(), lifeKept())
                    lifeDetailAgain()
                }
                redraws.request(drawingFinished, presented)
            }
        })
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow(); attached = true
        context.applicationContext.registerComponentCallbacks(memory)
        if (lifeOn) { lifeOpen(); life(CanvasLifeEvent.Attach) }
        controller.attachCanvas(::publish,retirementHook,::ensureRenderer,{retirementProgress.current})
        ensureRenderer()
    }
    override fun onDetachedFromWindow() {
        if (lifeOn) life(CanvasLifeEvent.Detach, lifeCurrent(), lifeKept(), generations.releaseCount.toLong())
        attached = false; cancelInput(); controller.detachCanvas(retirementHook)
        context.applicationContext.unregisterComponentCallbacks(memory)
        // A retirement can complete exceptionally only after AndroidX released the
        // renderer pipeline (its document reader close failed, and that frame keeps
        // its own quarantine). Confirm either way: an unconfirmed hook gates every
        // successor canvas forever and leaves the recreated canvas black.
        val detachedMs = SystemClock.uptimeMillis()
        retireRenderer(CanvasRetirementReason.WindowDetached).invokeOnCompletion{main.post{if(!attached){
            if (lifeOn) { val now = SystemClock.uptimeMillis(); life(CanvasLifeEvent.RetirementConfirmed, now - detachedMs, timeMs = now); lifeClose() }
            controller.canvasRendererRetired(retirementHook)
        }}}
        finishRedraws("detached")
        super.onDetachedFromWindow()
    }

    /** Main: releases every held SurfaceView redraw completion. */
    private fun finishRedraws(cause: String) {
        if (lifeOn && redraws.waiting > 0) life(CanvasLifeEvent.RedrawFinish, lifeWord(CanvasLifeWords.redrawCauses, cause), redraws.waiting.toLong())
        redraws.finish()
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
        val target = session ?: run { if (lifeOn) lifeUnownedScene(scene); return }
        if(target.latest?.documentFrame!==scene.documentFrame){retireRenderer(CanvasRetirementReason.DocumentFrameChanged);return}
        synchronized(lock) { if (!target.closed) target.latest = scene }
        enqueue(target, !wet(scene), "publish")
    }

    /** [cause] is a fixed word for the lifecycle diagnostics only. */
    private fun enqueue(target: RenderSession, dry: Boolean, cause: String) {
        val current = renderer ?: return
        val frame = synchronized(lock) {
            val made = if (target.closed || target !== session) null
                else if (dry) target.dryFrames.request()?.let { target.settledRedraw = false; Frame(target, true, lastInputMs, it) }
                else if (target.frontQueued) null
                else { target.frontQueued = true; Frame(target, false, lastInputMs) }
            // Ink frames are only counted, when drawn. Saved-frame requests are
            // recorded in full after a lifecycle event and counted otherwise.
            if (lifeOn && dry) lifeFrameRequest(target, made != null, cause)
            made
        }
        if (frame == null) return
        if (dry) current.renderMultiBufferedLayer(listOf(frame)) else current.renderFrontBufferedLayer(frame)
    }

    /** Main: tells the controller the surface's new size. No generation starts from
     * a scene it publishes on the way (see [CanvasGenerations.whileViewportChanges]). */
    private fun reportViewport(width: Int, height: Int, density: Float) {
        generations.whileViewportChanges { controller.viewport(width, height, density) }
    }

    /** Main, for an actual SurfaceView size change. */
    private fun resize(width: Int, height: Int) {
        val density = resources.displayMetrics.density
        val current = renderer; val currentSession = session
        val action = replacement.resizeAction(current != null && currentSession != null, controller.giantMode, generations.releasing)
        if (lifeOn) life(CanvasLifeEvent.Resize, action.ordinal.toLong(), width.toLong(), height.toLong(), lifeCurrent(),
            flag(currentSession?.presented == true), lifeKept(), generations.releaseCount.toLong())
        when (action) {
            CanvasReplacement.ResizeAction.RetireAll -> {
                retireRenderer(CanvasRetirementReason.SurfaceChanged); reportViewport(width, height, density)
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
                reportViewport(width, height, density); keptFollowsLatest()
            }
            CanvasReplacement.ResizeAction.Replace -> {
                val keptGeneration = Generation(checkNotNull(current), checkNotNull(currentSession))
                // AndroidX resizes the kept generation in place right after this
                // callback; its redraw must finish before that generation may leave.
                awaitResizeRedraw(keptGeneration.session)
                replacement.keep(keptGeneration, awaitRedraw = true)
                renderer = null; session = null
                // The controller cancels the tool in hand, and publishes, before it
                // takes the new size. The successor is created after it returns, so
                // its first frame is the scene for the new viewport, not that one.
                reportViewport(width, height, density)
                ensureRenderer()
                val successor = session ?: run { restoreKept(); return }
                keptFollowsLatest()
                // Wait (bounded) so the kept generation's AndroidX layer swap finds
                // the successor's frame already committed underneath.
                val started = SystemClock.uptimeMillis()
                val presented = successor.firstPresent.await(SUCCESSOR_PRESENT_WAIT_MS, TimeUnit.MILLISECONDS)
                Log.i(LOG_TAG, "resize successor_wait_ms=${SystemClock.uptimeMillis() - started} presented=$presented")
                if (lifeOn) {
                    val now = SystemClock.uptimeMillis()
                    life(CanvasLifeEvent.ResizeWait, successor.number.toLong(), keptGeneration.session.number.toLong(),
                        now - started, flag(presented), timeMs = now)
                }
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
        val retire = replacement.keptRedrawn(keptGeneration)
        if (lifeOn) life(CanvasLifeEvent.KeptRedrawn, target.number.toLong(), flag(retire != null))
        retire?.let(::retireKeptGeneration)
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
        if (lifeOn) life(CanvasLifeEvent.KeptRestored, target.session.number.toLong())
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
        if (lifeOn) life(CanvasLifeEvent.ReplaceAfterSettled, currentSession.number.toLong())
        renderer = null; session = null
        ensureRenderer()
        if (session == null) restoreKept()
    }

    private fun retireKeptGeneration(target: Generation) {
        main.removeCallbacks(keptFallback)
        release(target, CanvasRetirementReason.SurfaceChanged)
    }

    private fun ensureRenderer() {
        if (!attached || !controller.canCreateCanvas(retirementHook) || !holder.surface.isValid || width <= 0 || height <= 0 || surfaceWidth <= 0 || surfaceHeight <= 0 || latest == null || !generations.canCreate(renderer != null)) {
            if (lifeOn) lifeGenerationWaits()
            return
        }
        val currentSession = RenderSession(++generationNumbers).also { it.latest = latest;it.documentReader=latest?.documentFrame?.acquire() }
        if (lifeOn) {
            life(CanvasLifeEvent.GenerationCreated, currentSession.number.toLong(), surfaceWidth.toLong(), surfaceHeight.toLong(),
                width.toLong(), height.toLong(), lifeRev(latest), lifeViewWidth(latest), lifeViewHeight(latest), lifeKept(),
                lifeUnownedScenes.toLong())
            lifeWaitReason = null; lifeUnownedScenes = 0
        }
        session = currentSession
        renderer = try { CanvasFrontBufferedRenderer(this, object : CanvasFrontBufferedRenderer.Callback<Frame> {
            override fun onDrawFrontBufferedLayer(canvas: Canvas, bufferWidth: Int, bufferHeight: Int, param: Frame) {
                val snapshot = synchronized(lock) {
                    currentSession.frontQueued = false
                    if (lifeOn) currentSession.lifeBudget.inkFrame()
                    if (currentSession.closed || param.session !== currentSession) null else currentSession.latest?.also { currentSession.frontDrawn = it.revision;currentSession.giantFrontDrawn=it.giantFrame }
                } ?: return
                val drawStarted = System.nanoTime()
                // The kept picture with the stroke in progress over it. A scene
                // that cannot be drawn exactly that way is drawn whole.
                if (!drawKept(canvas, snapshot, currentSession)) drawOwned(canvas,snapshot,currentSession)
                // Timing only: two clock reads and a bounded array write.
                currentSession.frontSampleMs = snapshot.wetSampleMs
                controller.diagnostics.pen.inkDrawn(snapshot.wetSampleMs, snapshot.wetStartMs, SystemClock.uptimeMillis(), System.nanoTime() - drawStarted)
                if (param.inputMs > 0) main.post { if (!currentSession.closed) controller.drawCallback(param.inputMs) }
            }
            override fun onDrawMultiBufferedLayer(canvas: Canvas, bufferWidth: Int, bufferHeight: Int, params: Collection<Frame>) {
                val snapshot = synchronized(lock) {
                    val scene = if (currentSession.closed) null else {
                        currentSession.dryFrames.drawStarted(params.mapNotNull { it.dryRequest })
                        currentSession.latest?.also { currentSession.drawn = it.revision;currentSession.giantDrawn=it.giantFrame }
                    }
                    if (lifeOn) lifeDrawStarted(currentSession, scene, bufferWidth, bufferHeight, lifeRequests(params))
                    scene
                }
                if (snapshot == null) { canvas.drawColor(Color.TRANSPARENT, PorterDuff.Mode.CLEAR); return }
                val drawStarted = System.nanoTime()
                // The kept picture, with a stroke finished since drawn into it
                // once. A scene that cannot be drawn exactly that way is drawn whole.
                val kept = drawKept(canvas, snapshot, currentSession)
                if (!kept) drawOwned(canvas,snapshot,currentSession)
                // Timing only: two clock reads and a bounded array write.
                controller.diagnostics.pen.recordNanos(PenTiming.CanvasSavedDraw, System.nanoTime() - drawStarted)
                currentSession.savedDrawStartedNs = drawStarted
                val compare = synchronized(lock) { currentSession.compare.also { currentSession.compare = null } }
                if (compare != null) {
                    val compared = if (kept) currentSession.picture?.compare(snapshot) else null
                    main.post { compare(compared) }
                }
            }
            override fun onFrontBufferedLayerRenderComplete(frontBufferedLayerSurfaceControl: SurfaceControlCompat,
                                                            transaction: SurfaceControlCompat.Transaction) {
                controller.diagnostics.pen.frameReady(currentSession.frontSampleMs, SystemClock.uptimeMillis())
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
                val savedDrawStarted = currentSession.savedDrawStartedNs
                if (savedDrawStarted != 0L) {
                    currentSession.savedDrawStartedNs = 0L
                    controller.diagnostics.pen.recordNanos(PenTiming.CanvasSavedReady, System.nanoTime() - savedDrawStarted)
                }
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
                    val layerChanged = currentSession.lastMultiLayer !== multiBufferedLayerSurfaceControl
                    currentSession.lastMultiLayer = multiBufferedLayerSurfaceControl
                    val hidden = currentSession.closed || currentSession.giantDrawn?.let{!controller.giantFrameCurrent(it)}==true
                    if (hidden) {
                        transaction.setVisibility(frontBufferedLayerSurfaceControl, false)
                        transaction.setVisibility(multiBufferedLayerSurfaceControl, false)
                    }
                    if (lifeOn) lifeDrawCompleted(currentSession, !hidden, layerChanged, redrawnAfterResize)
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
        enqueue(currentSession, true, "created")
    }

    /** Any thread: the generation's first visible frame has been committed. */
    private fun presented(target: RenderSession) {
        if (target.presented) return
        target.firstPresent.countDown()
        // A commit callback: the record is one clock read and a write, nothing more.
        if (lifeOn) { val now = SystemClock.uptimeMillis(); life(CanvasLifeEvent.Presented, target.number.toLong(), now - target.createdMs, timeMs = now) }
        main.post {
            if (target !== session || target.closed) {
                if (lifeOn) life(CanvasLifeEvent.PresentedNotCurrent, target.number.toLong(), lifeCurrent())
                return@post
            }
            val retire = replacement.successorPresented()
            if (lifeOn && (retire != null || replacement.kept != null))
                life(CanvasLifeEvent.SuccessorPresented, target.number.toLong(), ((retire ?: replacement.kept)?.session?.number ?: 0).toLong(),
                    flag(retire != null))
            if (retire != null) retireKeptGeneration(retire)
            else if (replacement.kept != null) { main.removeCallbacks(keptFallback); main.postDelayed(keptFallback, KEPT_REDRAW_FALLBACK_MS) }
            finishRedraws("presented")
        }
    }

    private fun redrawNewer(target: RenderSession) {
        var settled = false
        val state = synchronized(lock) {
            if (target.closed || target !== session) null else target.latest?.let {
                val dry = !wet(it)
                if (it.revision > (if (dry) target.drawn else maxOf(target.drawn, target.frontDrawn))) dry
                // A frame owed since the kept pictures came to rest, asked for while another was in flight.
                else if (dry && target.settledRedraw) { settled = true; true }
                else null
            }
        }
        if (state != null) enqueue(target, state, if (settled) "settled" else "newer_scene")
    }

    private fun wet(scene: CanvasScene): Boolean =
        scene.provisional.isNotEmpty() || scene.transformed.isNotEmpty() || erasing(scene) || scene.documentDraft != null

    /** Whether saved objects are hidden for now only: an erasure in progress.
     * Objects a composed background already shows stay hidden for good. */
    private fun erasing(scene: CanvasScene): Boolean {
        if (scene.hidden.isEmpty()) return false
        val composed = (scene.document.hasToneAdjustments && scene.background != null) ||
            scene.giantFrame?.content == com.visualworkbench.shared.GiantTileContent.Composed
        return transientHiddenIds(scene.hidden, bakedCanonicalIds(scene.document, composed)).isNotEmpty()
    }

    /** Drawing thread: one frame from [target]'s kept picture, when [scene] can
     * be drawn exactly that way. False: nothing was drawn. */
    private fun drawKept(canvas: Canvas, scene: CanvasScene, target: RenderSession): Boolean {
        // A generation that owes the redraw of an in-place resize may be about
        // to leave, and is short of buffers while it does: it keeps no picture.
        val resizing = synchronized(lock) { target.awaitingResizeRedraw }
        // Once the view has rested and the pictures are drawn at it, a frame
        // that is the whole scene again is asked for.
        val picture = target.picture ?: KeptPicture(pictureCounts) {
            main.post { synchronized(lock) { target.settledRedraw = true }; enqueue(target, true, "settled") }
        }.also { target.picture = it }
        return picture.draw(canvas, scene, steady = !erasing(scene), resizing = resizing, first = !target.presented)
    }

    /** Main: every kept picture is given up; the next frame makes it again. */
    private fun dropPictures() {
        session?.picture?.requestDrop(); replacement.kept?.session?.picture?.requestDrop()
    }

    /** On-device tests only, Main: the next saved frame of the current
     * generation is also drawn whole, and [result] gets both as plain pixels;
     * null when that frame was not drawn from the kept picture. False: there
     * is no generation to ask. */
    internal fun comparePicture(result: (PictureComparison?) -> Unit): Boolean {
        val target = session ?: return false
        synchronized(lock) { if (target.closed) return false; target.compare = result }
        enqueue(target, true, "publish")
        return true
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
        if (lifeOn && (session != null || replacement.kept != null))
            life(CanvasLifeEvent.Retire, reason.ordinal.toLong(), lifeCurrent(), lifeKept())
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
        val requestedMs = SystemClock.uptimeMillis()
        // A release while frames are in flight, or while the in-place resize redraw is
        // still owed, also drops that generation's share of a held SurfaceView redraw.
        if (lifeOn) synchronized(lock) {
            life(CanvasLifeEvent.Release, owned.number.toLong(), reason.ordinal.toLong(), flag(owned.presented), owned.dryFrames.inFlight.toLong(),
                flag(owned.dryFrames.requestPending), flag(owned.awaitingResizeRedraw), owned.dryStarted.toLong(), owned.dryCompleted.toLong())
        }
        synchronized(lock) { owned.closed = true; owned.dryFrames.close() }
        val joined=generations.beginRelease()
        val release=retirementProgress.begin(reason)
        try {
            target.renderer.release(true){
                release.reached(CanvasRetirementMilestone.LibraryCallbackEntered)
                // The drawing thread: one clock read and a write, nothing more.
                if (lifeOn) { val now = SystemClock.uptimeMillis(); life(CanvasLifeEvent.ReleaseCallback, owned.number.toLong(), now - requestedMs, timeMs = now) }
                val posted=main.post{
                    release.reached(CanvasRetirementMilestone.MainCallbackEntered)
                    var readerClosed=false
                    try{
                        release.reached(CanvasRetirementMilestone.ReaderCloseRunning)
                        owned.picture?.discard();owned.picture=null
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
                    finally{
                        generations.settled(joined)
                        if (lifeOn) {
                            val now = SystemClock.uptimeMillis()
                            life(CanvasLifeEvent.ReleaseSettled, owned.number.toLong(), now - requestedMs, flag(readerClosed),
                                generations.releaseCount.toLong(), timeMs = now)
                        }
                        ensureRenderer();replaceAfterSettled()
                    }
                }
                if(!posted)release.failed(CanvasRetirementMilestone.MainCallbackEntered)
            }
        }catch(error:Throwable){release.failed(CanvasRetirementMilestone.ReleaseRequested);throw error}
    }

    // Lifecycle diagnostics. Everything below runs only while [lifeOn] (the test
    // hook [lifeState] aside). A record is one clock read and a write of numbers
    // into [CanvasLife]'s preallocated ring on whichever thread sees the event; the
    // log's own thread makes and writes the lines. Nothing here applies a
    // transaction, adds a listener or callback, or posts work, and nothing that
    // runs on the drawing thread or in a commit callback allocates.

    /** One record of [event]. On the drawing thread and in commit callbacks the
     * caller passes the [timeMs] it already read, so the record costs one clock read. */
    private fun life(event: CanvasLifeEvent, a: Long = 0, b: Long = 0, c: Long = 0, d: Long = 0, e: Long = 0, f: Long = 0,
                     g: Long = 0, h: Long = 0, i: Long = 0, j: Long = 0, k: Long = 0, l: Long = 0,
                     timeMs: Long = SystemClock.uptimeMillis()) =
        CanvasLife.record(event, surfaceNumber, timeMs, a, b, c, d, e, f, g, h, i, j, k, l)

    /** Main: see [lifeOpened]. A canvas attached again before its retirement was confirmed stays open. */
    private fun lifeOpen() { if (!lifeOpened) { lifeOpened = true; CanvasLife.opened() } }
    private fun lifeClose() { if (lifeOpened) { lifeOpened = false; CanvasLife.closed() } }

    private fun flag(value: Boolean): Long = if (value) 1 else 0
    private fun lifeRev(scene: CanvasScene?): Long = if (scene == null) 0 else scene.revision
    private fun lifeCurrent(): Long { val current = session; return if (current == null) 0 else current.number.toLong() }
    private fun lifeKept(): Long { val kept = replacement.kept; return if (kept == null) 0 else kept.session.number.toLong() }
    /** [word]'s position in [words], which is how a record carries it. */
    private fun lifeWord(words: List<String>, word: String): Long = words.indexOf(word).toLong()
    /** Drawing thread: how many of [params] carry an application request (without an iterator for a list). */
    private fun lifeRequests(params: Collection<Frame>): Int {
        if (params !is List<Frame>) return params.count { it.dryRequest != null }
        var count = 0
        for (index in params.indices) if (params[index].dryRequest != null) count++
        return count
    }

    /** The viewport size a scene was laid out for; negative (written `none`) without one. */
    private fun lifeViewWidth(scene: CanvasScene?): Long = if (scene == null) -1 else scene.camera.viewportWidth.toInt().toLong()
    private fun lifeViewHeight(scene: CanvasScene?): Long = if (scene == null) -1 else scene.camera.viewportHeight.toInt().toLong()
    /** The same, as text, for [lifeState]. */
    private fun lifeViewOf(scene: CanvasScene?): String =
        if (scene == null) "none" else "${scene.camera.viewportWidth.toInt()}x${scene.camera.viewportHeight.toInt()}"

    /** Main, under [lock]: records asking a saved frame of [target]'s newest scene, or
     * a summary of the requests only counted, or nothing. */
    private fun lifeFrameRequest(target: RenderSession, granted: Boolean, cause: String) {
        val now = SystemClock.uptimeMillis()
        if (!target.lifeBudget.request(granted)) { lifeQuietSummaryDue(target, now); return }
        val result = when {
            granted -> "granted"
            target.closed -> "closed"
            target !== session -> "not_current"
            target.dryFrames.requestPending -> "request_pending"
            else -> "draw_in_flight"
        }
        val scene = target.latest
        life(CanvasLifeEvent.FrameRequest, target.number.toLong(), lifeWord(CanvasLifeWords.frameCauses, cause), lifeRev(scene),
            lifeViewWidth(scene), lifeViewHeight(scene), lifeWord(CanvasLifeWords.frameResults, result),
            target.dryFrames.inFlight.toLong(), timeMs = now)
    }

    /** Under [lock]: records what [target] only counted, at most once a second. */
    private fun lifeQuietSummaryDue(target: RenderSession, now: Long) {
        if (target.lifeBudget.summaryDue(now)) lifeQuiet(target, now)
    }

    /** Under [lock]: records the counts [target]'s budget has just taken. */
    private fun lifeQuiet(target: RenderSession, now: Long) {
        val budget = target.lifeBudget
        life(CanvasLifeEvent.Quiet, target.number.toLong(), target.dryCompleted.toLong(), target.dryStarted.toLong(), target.drawn,
            lifeRev(target.latest), target.dryFrames.inFlight.toLong(), budget.takenRequests.toLong(), budget.takenGranted.toLong(),
            budget.takenFrames.toLong(), budget.takenInk.toLong(), timeMs = now)
    }

    /** Main, at a lifecycle event: the next frames of every live generation are
     * recorded in full again, after what was only counted until now. */
    private fun lifeDetailAgain() {
        for (target in listOfNotNull(session, replacement.kept?.session)) synchronized(lock) {
            val now = SystemClock.uptimeMillis()
            if (target.lifeBudget.again(target.dryStarted, now)) lifeQuiet(target, now)
        }
    }

    /** Drawing thread, under [lock]: a saved frame of [scene] is being recorded.
     * Counts, and one clock read and a write when the frame is recorded in full. */
    private fun lifeDrawStarted(target: RenderSession, scene: CanvasScene?, bufferWidth: Int, bufferHeight: Int, requests: Int) {
        target.dryStarted++
        if (scene != null) {
            target.drawnViewWidth = scene.camera.viewportWidth.toInt(); target.drawnViewHeight = scene.camera.viewportHeight.toInt()
        }
        if (!target.lifeBudget.frameWritten(target.dryStarted)) return
        life(CanvasLifeEvent.Draw, target.number.toLong(), target.dryStarted.toLong(), lifeRev(scene), lifeViewWidth(scene),
            lifeViewHeight(scene), bufferWidth.toLong(), bufferHeight.toLong(), requests.toLong(), target.dryFrames.inFlight.toLong(),
            lifeRev(target.latest), target.lifeBudget.takeInk().toLong(), flag(target.closed))
    }

    /** Drawing thread, under [lock], before AndroidX commits the frame: a saved frame's
     * buffer is ready and goes to the compositor. Counts and one clock read, and a
     * write when the frame (or a due summary) is recorded. */
    private fun lifeDrawCompleted(target: RenderSession, visible: Boolean, layerChanged: Boolean, resizeRedraw: Boolean) {
        target.dryCompleted++
        val now = SystemClock.uptimeMillis()
        if (!target.lifeBudget.frameWritten(target.dryCompleted)) {
            target.lifeBudget.quietFrame(); lifeQuietSummaryDue(target, now); return
        }
        life(CanvasLifeEvent.FrameReady, target.number.toLong(), target.dryCompleted.toLong(), target.dryStarted.toLong(), flag(visible),
            flag(layerChanged), flag(resizeRedraw), target.dryFrames.inFlight.toLong(), flag(target.dryFrames.requestPending),
            target.drawn, lifeRev(target.latest), timeMs = now)
    }

    /** Main: why no generation can be created just now; recorded when the reason changes. */
    private fun lifeGenerationWaits() {
        if (renderer != null) return // A current generation is not a wait.
        val reason = when {
            !attached -> "detached"
            latest == null -> "no_scene"
            !holder.surface.isValid || width <= 0 || height <= 0 || surfaceWidth <= 0 || surfaceHeight <= 0 -> "no_surface"
            generations.viewportChanging -> "viewport_changing"
            generations.releasing -> "release_settling"
            !controller.canCreateCanvas(retirementHook) -> "controller_gate"
            else -> "other"
        }
        if (reason == lifeWaitReason) return
        lifeWaitReason = reason
        val awaited = controller.canvasRetirementProgress
        // Retirement and milestone are carried as ordinal + 1, with 0 for none.
        life(CanvasLifeEvent.GenerationWaits, lifeWord(CanvasLifeWords.waitReasons, reason), generations.releaseCount.toLong(),
            (if (awaited == null) 0 else awaited.reason.ordinal + 1).toLong(), (if (awaited == null) 0 else awaited.milestone.ordinal + 1).toLong())
    }

    /** Main: a scene arrived while no generation exists. The first few of a run are recorded, the rest counted. */
    private fun lifeUnownedScene(scene: CanvasScene) {
        if (lifeUnownedScenes++ < 4) life(CanvasLifeEvent.SceneWithoutGeneration, scene.revision, lifeViewWidth(scene), lifeViewHeight(scene))
    }

    /** On-screen tests, Main: this canvas's own account of what it has drawn, in fixed
     * words and numbers. Reads state only. Frame counts and the drawn viewport are
     * kept only while the lifecycle diagnostics are switched on. */
    internal fun lifeState(): String {
        fun generation(name: String, target: RenderSession?): String = if (target == null) "$name=0" else synchronized(lock) {
            "$name=${target.number} ${name}_presented=${target.presented} ${name}_drawn_rev=${target.drawn} " +
                "${name}_drawn_view=${target.drawnViewWidth}x${target.drawnViewHeight} ${name}_newest_rev=${target.latest?.revision ?: 0} " +
                "${name}_in_flight=${target.dryFrames.inFlight} ${name}_request_pending=${target.dryFrames.requestPending} " +
                "${name}_resize_redraw_pending=${target.awaitingResizeRedraw} ${name}_frames=${target.dryCompleted}/${target.dryStarted}"
        }
        return "surface=$surfaceNumber attached=$attached size=${surfaceWidth}x$surfaceHeight view=${width}x$height " +
            "newest_rev=${latest?.revision ?: 0} newest_view=${lifeViewOf(latest)} ${generation("current", session)} " +
            "${generation("kept", replacement.kept?.session)} replace_deferred=${replacement.replaceDeferred} " +
            "releases=${generations.releaseCount} redraws_held=${redraws.waiting} can_create=${controller.canCreateCanvas(retirementHook)}"
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
        /** Lifecycle diagnostics: saved frames and frame requests recorded in full
         * after each lifecycle event, and the shortest gap between two summaries. */
        const val LIFE_DETAIL_FRAMES = 8
        const val LIFE_DETAIL_REQUESTS = 16
        const val LIFE_SUMMARY_MS = 1_000L
    }
}
