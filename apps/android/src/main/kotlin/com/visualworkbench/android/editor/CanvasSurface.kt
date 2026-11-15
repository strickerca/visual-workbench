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
import android.view.SurfaceControl
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
import java.util.concurrent.atomic.AtomicInteger

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
 * Lifecycle diagnostics ([CanvasLife]) are written only when switched on, and a
 * surface reads that switch once, when it is constructed. Ink frames are only
 * counted. Saved frames and their requests, which hover, pan and zoom produce per
 * input sample, are written in full only for a few frames after each lifecycle
 * event and otherwise counted, with at most one summary line a second
 * ([CanvasLifeBudget]). */
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
        /** Diagnostics only: the pen sample time of the last drawn ink frame. */
        @Volatile var frontSampleMs = 0L
        /** Developer switch only: the kept picture an ink frame draws over. Made and
         * used on the drawing thread; discarded on Main once that thread has stopped. */
        @Volatile var wetInk: WetInkLayer? = null
        /** Lifecycle diagnostics only, under [lock] and only while switched on: saved
         * frames started and completed, which frame lines are written, and the
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
        if (lifeOn) life("kept_fallback gen=${waiting?.session?.number ?: 0} retired=${retire != null} " +
            "resize_redraw_pending=${waiting?.let { synchronized(lock) { it.session.awaitingResizeRedraw } } == true}")
        retire?.let { release(it, CanvasRetirementReason.SurfaceChanged) }
    }
    private val generations = CanvasGenerations()
    private val retirementProgress=CanvasRetirementProgress()
    private val redraws = CanvasRedrawHandshake({ task, delay -> main.postDelayed(task, delay) }, { main.removeCallbacks(it) }, REDRAW_FALLBACK_MS,
        { held -> if (lifeOn) life("redraw_fallback held=$held current=${session?.number ?: 0} current_presented=${session?.presented == true}") })
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

    init {
        contentDescription = "Document canvas. Pen draws; one finger pans; two fingers zoom and rotate."
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
        isFocusable = true
        holder.addCallback(object : SurfaceHolder.Callback2 {
            override fun surfaceCreated(holder: SurfaceHolder) {
                if (lifeOn) life("surface_created view=${this@CanvasSurface.width}x${this@CanvasSurface.height}")
                ensureRenderer()
            }
            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
                val changed = surfaceWidth != width || surfaceHeight != height
                if (lifeOn) { life("surface_changed size=${width}x$height changed=$changed"); lifeWatchWindowFrame(); lifeDetailAgain() }
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
                if (lifeOn) life("surface_destroyed")
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
                    life("redraw_request presented=$presented held=${redraws.waiting} " +
                        "current=${session?.number ?: 0} kept=${replacement.kept?.session?.number ?: 0}")
                    lifeDetailAgain()
                }
                redraws.request(drawingFinished, presented)
            }
        })
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow(); attached = true
        if (lifeOn) life("attach")
        controller.attachCanvas(::publish,retirementHook,::ensureRenderer,{retirementProgress.current})
        ensureRenderer()
    }
    override fun onDetachedFromWindow() {
        if (lifeOn) life("detach current=${session?.number ?: 0} kept=${replacement.kept?.session?.number ?: 0} releases=${generations.releaseCount}")
        attached = false; cancelInput(); controller.detachCanvas(retirementHook)
        // A retirement can complete exceptionally only after AndroidX released the
        // renderer pipeline (its document reader close failed, and that frame keeps
        // its own quarantine). Confirm either way: an unconfirmed hook gates every
        // successor canvas forever and leaves the recreated canvas black.
        val detachedMs = SystemClock.uptimeMillis()
        retireRenderer(CanvasRetirementReason.WindowDetached).invokeOnCompletion{main.post{if(!attached){
            if (lifeOn) life("retirement_confirmed after_detach_ms=${SystemClock.uptimeMillis() - detachedMs}")
            controller.canvasRendererRetired(retirementHook)
        }}}
        finishRedraws("detached")
        super.onDetachedFromWindow()
    }

    /** Main: releases every held SurfaceView redraw completion. */
    private fun finishRedraws(cause: String) {
        if (lifeOn && redraws.waiting > 0) life("redraw_finish cause=$cause released=${redraws.waiting}")
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
        var note: String? = null
        val frame = synchronized(lock) {
            val made = if (target.closed || target !== session) null
                else if (dry) target.dryFrames.request()?.let { Frame(target, true, lastInputMs, it) }
                else if (target.frontQueued) null
                else { target.frontQueued = true; Frame(target, false, lastInputMs) }
            // Ink frames are only counted, when drawn. Saved-frame requests are
            // written in full after a lifecycle event and counted otherwise.
            if (lifeOn && dry) note = lifeFrameRequest(target, made != null, cause)
            made
        }
        note?.let { life(it) }
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
        if (lifeOn) life("resize action=${action.name} size=${width}x$height current=${currentSession?.number ?: 0} " +
            "current_presented=${currentSession?.presented == true} kept=${replacement.kept?.session?.number ?: 0} " +
            "releases=${generations.releaseCount}")
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
                if (lifeOn) life("resize_wait gen=${successor.number} kept=${keptGeneration.session.number} " +
                    "waited_ms=${SystemClock.uptimeMillis() - started} presented=$presented")
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
        if (lifeOn) life("kept_redrawn gen=${target.number} retires=${retire != null}")
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
        if (lifeOn) life("kept_restored gen=${target.session.number}")
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
        if (lifeOn) life("replace_after_settled kept=${currentSession.number}")
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
            life("generation_created gen=${currentSession.number} size=${surfaceWidth}x$surfaceHeight view=${width}x$height " +
                "rev=${latest?.revision ?: 0} scene_view=${lifeViewOf(latest)} kept=${replacement.kept?.session?.number ?: 0} " +
                "scenes_without_generation=$lifeUnownedScenes")
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
                // Developer switch: draw the kept picture and only the stroke in
                // progress. Any scene it cannot draw exactly falls through.
                if (wholeSceneNeeded(snapshot.keepDryPicture) { (currentSession.wetInk ?: WetInkLayer().also { currentSession.wetInk = it }).draw(canvas, snapshot) })
                    drawOwned(canvas,snapshot,currentSession)
                // Timing only: two clock reads and a bounded array write.
                currentSession.frontSampleMs = snapshot.wetSampleMs
                controller.diagnostics.pen.inkDrawn(snapshot.wetSampleMs, snapshot.wetStartMs, SystemClock.uptimeMillis(), System.nanoTime() - drawStarted)
                if (param.inputMs > 0) main.post { if (!currentSession.closed) controller.drawCallback(param.inputMs) }
            }
            override fun onDrawMultiBufferedLayer(canvas: Canvas, bufferWidth: Int, bufferHeight: Int, params: Collection<Frame>) {
                var note: String? = null
                val snapshot = synchronized(lock) {
                    val scene = if (currentSession.closed) null else {
                        currentSession.dryFrames.drawStarted(params.mapNotNull { it.dryRequest })
                        currentSession.latest?.also { currentSession.drawn = it.revision;currentSession.giantDrawn=it.giantFrame }
                    }
                    if (lifeOn) note = lifeDrawStarted(currentSession, scene, bufferWidth, bufferHeight, params.count { it.dryRequest != null })
                    scene
                }
                note?.let { life(it) }
                // Developer switch only: the kept picture is held while ink is wet, never between strokes.
                currentSession.wetInk?.forget()
                if (snapshot == null) canvas.drawColor(Color.TRANSPARENT, PorterDuff.Mode.CLEAR)
                else drawOwned(canvas,snapshot,currentSession)
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
                var note: String? = null; var quiet: String? = null; var completedFrame = 0
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
                    if (lifeOn) {
                        note = lifeDrawCompleted(currentSession, !hidden, layerChanged, redrawnAfterResize)
                        if (note == null) quiet = lifeQuietSummaryDue(currentSession)
                        completedFrame = currentSession.dryCompleted
                    }
                    !hidden
                }
                quiet?.let { life(it) }
                val ready = note
                if (ready != null) {
                    life(ready)
                    // Diagnostics only: when the compositor took this frame, to tell a
                    // slow drawing thread from a frame held back after it was handed
                    // over. From API 33 the library uses the platform transaction,
                    // which keeps every committed listener (each gets its own callback),
                    // so the first-presentation listener below still runs. Before that
                    // it goes through the NDK, so this one is not added there.
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                        val committedFrame = completedFrame; val handedOver = SystemClock.uptimeMillis()
                        transaction.addTransactionCommittedListener({ it.run() }, object : SurfaceControlCompat.TransactionCommittedListener {
                            override fun onTransactionCommitted() {
                                life("frame_committed gen=${currentSession.number} frame=$committedFrame after_ms=${SystemClock.uptimeMillis() - handedOver}")
                            }
                        })
                    }
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
        if (lifeOn) life("presented gen=${target.number} after_created_ms=${SystemClock.uptimeMillis() - target.createdMs}")
        main.post {
            if (target !== session || target.closed) {
                if (lifeOn) life("presented_not_current gen=${target.number} current=${session?.number ?: 0}")
                return@post
            }
            val retire = replacement.successorPresented()
            if (lifeOn && (retire != null || replacement.kept != null))
                life("successor_presented gen=${target.number} kept=${(retire ?: replacement.kept)?.session?.number ?: 0} kept_retires=${retire != null}")
            if (retire != null) retireKeptGeneration(retire)
            else if (replacement.kept != null) { main.removeCallbacks(keptFallback); main.postDelayed(keptFallback, KEPT_REDRAW_FALLBACK_MS) }
            finishRedraws("presented")
        }
    }

    private fun redrawNewer(target: RenderSession) {
        val state = synchronized(lock) {
            if (target.closed || target !== session) null else target.latest?.let {
                val dry = !wet(it)
                if (it.revision > (if (dry) target.drawn else maxOf(target.drawn, target.frontDrawn))) dry else null
            }
        }
        if (state != null) enqueue(target, state, "newer_scene")
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
        if (lifeOn && (session != null || replacement.kept != null))
            life("retire reason=${reason.name} current=${session?.number ?: 0} kept=${replacement.kept?.session?.number ?: 0}")
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
        if (lifeOn) life("release gen=${owned.number} reason=${reason.name} " + synchronized(lock) {
            "presented=${owned.presented} in_flight=${owned.dryFrames.inFlight} request_pending=${owned.dryFrames.requestPending} " +
                "resize_redraw_pending=${owned.awaitingResizeRedraw} started=${owned.dryStarted} completed=${owned.dryCompleted}" })
        synchronized(lock) { owned.closed = true; owned.dryFrames.close() }
        val joined=generations.beginRelease()
        val release=retirementProgress.begin(reason)
        try {
            target.renderer.release(true){
                release.reached(CanvasRetirementMilestone.LibraryCallbackEntered)
                if (lifeOn) life("release_callback gen=${owned.number} after_ms=${SystemClock.uptimeMillis() - requestedMs}")
                val posted=main.post{
                    release.reached(CanvasRetirementMilestone.MainCallbackEntered)
                    var readerClosed=false
                    try{
                        release.reached(CanvasRetirementMilestone.ReaderCloseRunning)
                        owned.wetInk?.discard();owned.wetInk=null
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
                        if (lifeOn) life("release_settled gen=${owned.number} after_ms=${SystemClock.uptimeMillis() - requestedMs} " +
                            "reader_closed=$readerClosed releases=${generations.releaseCount}")
                        ensureRenderer();replaceAfterSettled()
                    }
                }
                if(!posted)release.failed(CanvasRetirementMilestone.MainCallbackEntered)
            }
        }catch(error:Throwable){release.failed(CanvasRetirementMilestone.ReleaseRequested);throw error}
    }

    // Lifecycle diagnostics. Everything below runs only while [lifeOn] (the test
    // hook [lifeState] aside) and writes fixed words and numbers, nothing else.

    private fun life(text: String) { CanvasLife.line("surface=$surfaceNumber $text") }

    /** The viewport size a scene was laid out for. */
    private fun lifeViewOf(scene: CanvasScene?): String =
        if (scene == null) "none" else "${scene.camera.viewportWidth.toInt()}x${scene.camera.viewportHeight.toInt()}"

    /** Main, under [lock]: the line for asking a saved frame of [target]'s newest
     * scene, or a summary of the requests only counted, or nothing. */
    private fun lifeFrameRequest(target: RenderSession, granted: Boolean, cause: String): String? {
        if (!target.lifeBudget.request(granted)) return lifeQuietSummaryDue(target)
        val result = when {
            granted -> "granted"
            target.closed -> "closed"
            target !== session -> "not_current"
            target.dryFrames.requestPending -> "request_pending"
            else -> "draw_in_flight"
        }
        return "frame_request gen=${target.number} cause=$cause rev=${target.latest?.revision ?: 0} " +
            "scene_view=${lifeViewOf(target.latest)} result=$result in_flight=${target.dryFrames.inFlight}"
    }

    /** Under [lock]: one line with what [target] only counted, at most once a second. */
    private fun lifeQuietSummaryDue(target: RenderSession): String? =
        target.lifeBudget.summaryDue(SystemClock.uptimeMillis())?.let { lifeQuietLine(target, it) }

    private fun lifeQuietLine(target: RenderSession, counts: String): String =
        "quiet gen=${target.number} frames=${target.dryCompleted}/${target.dryStarted} drawn_rev=${target.drawn} " +
            "newest_rev=${target.latest?.revision ?: 0} in_flight=${target.dryFrames.inFlight} $counts"

    /** Main, at a lifecycle event: the next frames of every live generation are
     * written in full again, after what was only counted until now. */
    private fun lifeDetailAgain() {
        for (target in listOfNotNull(session, replacement.kept?.session)) {
            val counts = synchronized(lock) {
                target.lifeBudget.again(target.dryStarted, SystemClock.uptimeMillis())?.let { lifeQuietLine(target, it) }
            }
            counts?.let { life(it) }
        }
    }

    /** Drawing thread, under [lock]: a saved frame of [scene] is being recorded.
     * The line, or null when this frame is only counted. */
    private fun lifeDrawStarted(target: RenderSession, scene: CanvasScene?, bufferWidth: Int, bufferHeight: Int, requests: Int): String? {
        target.dryStarted++
        if (scene != null) {
            target.drawnViewWidth = scene.camera.viewportWidth.toInt(); target.drawnViewHeight = scene.camera.viewportHeight.toInt()
        }
        if (!target.lifeBudget.frameWritten(target.dryStarted)) return null
        return "draw gen=${target.number} frame=${target.dryStarted} rev=${scene?.revision ?: 0} scene_view=${lifeViewOf(scene)} " +
            "buffer=${bufferWidth}x$bufferHeight requests=$requests in_flight=${target.dryFrames.inFlight} " +
            "newest_rev=${target.latest?.revision ?: 0} ink_frames=${target.lifeBudget.takeInk()} closed=${target.closed}"
    }

    /** Drawing thread, under [lock]: a saved frame's buffer is ready and goes to the
     * compositor. The line, or null when this frame is only counted. */
    private fun lifeDrawCompleted(target: RenderSession, visible: Boolean, layerChanged: Boolean, resizeRedraw: Boolean): String? {
        target.dryCompleted++
        if (!target.lifeBudget.frameWritten(target.dryCompleted)) { target.lifeBudget.quietFrame(); return null }
        return "frame_ready gen=${target.number} frame=${target.dryCompleted} started=${target.dryStarted} visible=$visible " +
            "new_layer=$layerChanged resize_redraw=$resizeRedraw in_flight=${target.dryFrames.inFlight} " +
            "request_pending=${target.dryFrames.requestPending} drawn_rev=${target.drawn} newest_rev=${target.latest?.revision ?: 0}"
    }

    /** Main: why no generation can be created just now; written when the reason changes. */
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
        life("generation_waits reason=$reason releases=${generations.releaseCount} " +
            "awaited_retirement=${awaited?.reason?.name ?: "None"} awaited_milestone=${awaited?.milestone?.name ?: "None"}")
    }

    /** Main: a scene arrived while no generation exists. The first few of a run are written, the rest counted. */
    private fun lifeUnownedScene(scene: CanvasScene) {
        if (lifeUnownedScenes++ < 4) life("scene_without_generation rev=${scene.revision} scene_view=${lifeViewOf(scene)}")
    }

    /** Main: when the window's own next frame has been drawn, and when the compositor
     * has taken it, to tell a late canvas from a late window. The second rides on
     * the window's frame the way SurfaceView's own geometry change does. */
    private fun lifeWatchWindowFrame() {
        val asked = SystemClock.uptimeMillis()
        try {
            viewTreeObserver.registerFrameCommitCallback { life("window_frame_drawn after_surface_changed_ms=${SystemClock.uptimeMillis() - asked}") }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) rootSurfaceControl?.let { root ->
                val shown = SurfaceControl.Transaction()
                shown.addTransactionCommittedListener({ it.run() }) {
                    life("window_frame_shown after_surface_changed_ms=${SystemClock.uptimeMillis() - asked}")
                }
                root.applyTransactionOnDraw(shown)
            }
        } catch (_: IllegalStateException) { /* The observer left with its window: nothing to watch. */ }
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
        /** Lifecycle diagnostics: saved frames and frame requests written in full
         * after each lifecycle event, and the shortest gap between two summaries. */
        const val LIFE_DETAIL_FRAMES = 8
        const val LIFE_DETAIL_REQUESTS = 16
        const val LIFE_SUMMARY_MS = 1_000L
    }
}

/** Lifecycle diagnostics of the canvas surface, for telling a slow device from a
 * redraw the canvas lost. One log tag; fixed words and numbers only (ordinal
 * numbers of surfaces, generations, frames and scene revisions, pixel sizes,
 * milliseconds, state names), never content, paths or identifiers.
 *
 * Off unless switched on for this process before the canvas is created: on a
 * device with `adb shell setprop log.tag.VW_CANVAS_LIFE VERBOSE` (the setting
 * lasts until the device restarts), or by an on-screen test through [forced].
 * While off, no line is built and nothing is kept. While on, every saved frame
 * writes four lines, so it is for a lifecycle check, not for everyday use. */
internal object CanvasLife {
    const val TAG = "VW_CANVAS_LIFE"
    /** On-screen tests only. */
    @Volatile var forced = false
    private val surfaces = AtomicInteger()
    private val history = CanvasLifeHistory(240)

    fun enabled(): Boolean = forced || Log.isLoggable(TAG, Log.VERBOSE)
    fun nextSurface(): Int = surfaces.incrementAndGet()

    /** Any thread. `t` is the device's uptime clock in milliseconds. */
    fun line(text: String) {
        val stamped = "t=${SystemClock.uptimeMillis()} $text"
        history.add(stamped)
        Log.i(TAG, stamped)
    }

    /** The newest lines, oldest first. */
    fun recent(): List<String> = history.snapshot()
}
