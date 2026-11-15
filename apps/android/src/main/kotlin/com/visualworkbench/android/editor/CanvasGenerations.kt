package com.visualworkbench.android.editor

import kotlinx.coroutines.CompletableDeferred

/** Main-confined release bookkeeping for one canvas surface's renderer generations.
 *
 * A generation is one AndroidX renderer with its buffer pools and scene slot. No
 * generation is allocated while any release is still settling, or between the
 * surface taking a new size and the scene's owner having been told it. Releases
 * that overlap in time form one batch; [joined] completes only after every release of
 * the batch has settled and then fails with the batch's first failure. */
internal class CanvasGenerations {
    private val releases = LinkedHashSet<CompletableDeferred<Unit>>()
    private var batch: CompletableDeferred<Unit>? = null
    private var batchFailure: Throwable? = null
    private var viewportChanges = 0
    val releasing: Boolean get() = releases.isNotEmpty()
    val releaseCount: Int get() = releases.size
    /** The surface has a new size that the scene's owner has not finished taking. */
    val viewportChanging: Boolean get() = viewportChanges > 0

    /** A fresh generation needs no current one, no release still settling and no
     * viewport change under way (see [whileViewportChanges]). */
    fun canCreate(hasCurrent: Boolean): Boolean = !hasCurrent && releases.isEmpty() && viewportChanges == 0

    /** Runs [step], one step of the surface taking a new size: cancelling the input
     * in hand, or telling the scene's owner the size. Either makes the owner
     * publish before it has the new size (it cancels the tool first), and such a
     * scene still carries the previous viewport: a generation created from it would
     * draw its first frame laid out for the old size, and on a surface that had
     * none it would be kept while a second one is built. So none is created until
     * every step has returned; the caller then creates one from the newest scene. */
    fun <T> whileViewportChanges(step: () -> T): T {
        viewportChanges++
        try { return step() } finally { viewportChanges-- }
    }

    /** Registers an actual renderer release. The token completes on Main once the
     * release callback has closed the generation's reader; then call [settled]. */
    fun beginRelease(): CompletableDeferred<Unit> {
        if (releases.isEmpty()) { batch = CompletableDeferred(); batchFailure = null }
        return CompletableDeferred<Unit>().also { releases += it }
    }

    /** Main, after [token] completed (successfully or not). Unknown or still
     * incomplete tokens are ignored, so a release that never completes keeps
     * its batch, and every caller joined to it, outstanding. */
    fun settled(token: CompletableDeferred<Unit>) {
        if (!token.isCompleted || !releases.remove(token)) return
        var cause: Throwable? = null
        token.invokeOnCompletion { cause = it } // Already complete: runs synchronously.
        if (batchFailure == null) batchFailure = cause
        if (releases.isNotEmpty()) return
        val done = batch ?: return
        val failure = batchFailure
        if (failure == null) done.complete(Unit) else done.completeExceptionally(failure)
    }

    /** The current (or, with nothing outstanding, the most recent) release batch. */
    fun joined(): CompletableDeferred<Unit> = batch ?: CompletableDeferred(Unit)
}

/** Main-confined make-before-break decisions for one canvas surface, independent
 * of AndroidX so the state machine is unit-testable.
 *
 * On a size change the presented generation is kept while one successor is
 * created. The kept generation is retired only when both hold: the successor's
 * first frame is committed, and the kept generation's own AndroidX resize redraw
 * has completed. The latter carries SurfaceView's redraw completion; quitting its
 * handler thread earlier would drop it. A bounded fallback retires the kept
 * generation once the successor presented even if that redraw never completes. */
internal class CanvasReplacement<G : Any> {
    enum class ResizeAction { RetireAll, Replace, Defer }

    /** The presented generation kept while its successor prepares its first frame. */
    var kept: G? = null
        private set
    /** A resize arrived while a replacement was settling; one coalesced replacement follows. */
    var replaceDeferred: Boolean = false
        private set
    private var successorPresented = false
    private var keptRedrawn = true

    /** Giant mode and a missing renderer keep break-before-make. A resize during an
     * unsettled replacement is deferred: presented generations resize in place
     * through AndroidX and the kept generation must redraw again before it leaves. */
    fun resizeAction(hasCurrent: Boolean, giant: Boolean, releasing: Boolean): ResizeAction = when {
        !hasCurrent || giant -> ResizeAction.RetireAll
        kept != null || releasing -> {
            replaceDeferred = true
            if (kept != null) keptRedrawn = false
            ResizeAction.Defer
        }
        else -> ResizeAction.Replace
    }

    /** [current] becomes kept. [awaitRedraw] when its AndroidX resize redraw is still pending. */
    fun keep(current: G, awaitRedraw: Boolean) {
        check(kept == null) { "A replacement is already in progress" }
        kept = current; successorPresented = false; keptRedrawn = !awaitRedraw
    }

    /** No successor could be allocated: the kept generation becomes current again. */
    fun restore(): G? = take()

    /** The current successor committed its first visible frame. Returns the kept generation to retire now. */
    fun successorPresented(): G? {
        if (kept == null) return null
        successorPresented = true
        return ready()
    }

    /** [generation]'s AndroidX resize redraw completed. Returns the kept generation to retire now. */
    fun keptRedrawn(generation: G): G? {
        if (kept !== generation) return null
        keptRedrawn = true
        return ready()
    }

    /** Bounded fallback after the successor presented. */
    fun fallback(): G? = if (kept != null && successorPresented) take() else null

    /** Full retirement takes the kept generation unconditionally. */
    fun take(): G? = kept.also { kept = null; successorPresented = false; keptRedrawn = true }

    /** A release settled: true exactly once per deferred run of resizes. */
    fun replaceDue(releasing: Boolean): Boolean {
        if (!replaceDeferred || releasing || kept != null) return false
        replaceDeferred = false
        return true
    }

    fun clearDeferred() { replaceDeferred = false }

    private fun ready(): G? = if (successorPresented && keptRedrawn) take() else null
}

/** SurfaceView cuts its window hole once every redraw callback has finished. A
 * canvas with no committed frame would then expose SurfaceView's opaque black
 * background, so finishing waits for this canvas's first committed frame, bounded
 * by one fallback (not restarted by later requests) so a gated successor can
 * never stall a window sync indefinitely. */
internal class CanvasRedrawHandshake(
    private val schedule: (Runnable, Long) -> Unit,
    private val unschedule: (Runnable) -> Unit,
    private val fallbackMs: Long,
    /** Diagnostics only: the bounded fallback fired with this many redraws still held. */
    private val fallbackFired: (Int) -> Unit = {},
) {
    private val pending = ArrayList<Runnable>()
    private val fallback = Runnable { fallbackFired(pending.size); finish() }
    val waiting: Int get() = pending.size

    fun request(drawingFinished: Runnable, presented: Boolean) {
        if (presented) { drawingFinished.run(); return }
        if (pending.isEmpty()) schedule(fallback, fallbackMs)
        pending += drawingFinished
    }

    fun finish() {
        unschedule(fallback)
        if (pending.isEmpty()) return
        val ready = pending.toList(); pending.clear()
        for (callback in ready) callback.run()
    }
}

/** The newest lifecycle lines of the canvas, kept in memory only while that
 * diagnostic is switched on, so an on-screen test can put them in its failure
 * message after the device log has rolled over. Lines arrive from Main and from
 * each generation's drawing thread. */
internal class CanvasLifeHistory(private val capacity: Int) {
    private val lines = ArrayDeque<String>()
    init { require(capacity > 0) }

    fun add(line: String) {
        synchronized(lines) { if (lines.size == capacity) lines.removeFirst(); lines.addLast(line) }
    }

    fun snapshot(): List<String> = synchronized(lines) { lines.toList() }
}

/** Lifecycle diagnostics only: which per-frame lines of one canvas generation are
 * written. Hovering, panning and zooming publish a saved-frame scene per input
 * sample, so writing each would flood the log and the in-memory history. After
 * each lifecycle event ([again]) the next [frames] saved frames and [requests]
 * frame requests are written in full; beyond that they are only counted, and at
 * most one summary per [summaryMs] reports the counts. Frames are numbered from 1
 * in the order they start and, separately, in the order they complete; both
 * orders agree, so a frame whose start was written also has its completion
 * written. The caller holds the generation's lock. */
internal class CanvasLifeBudget(private val frames: Int, private val requests: Int, private val summaryMs: Long) {
    private var framesUntil = frames
    private var requestsLeft = requests
    private var quietRequests = 0
    private var quietGranted = 0
    private var quietFrames = 0
    private var inkFrames = 0
    private var lastSummaryMs: Long? = null
    init { require(frames >= 0 && requests >= 0 && summaryMs > 0) }

    /** A lifecycle event: frames after [started] and the next requests are written
     * again. Returns the counts gathered since the last summary, if any. */
    fun again(started: Int, nowMs: Long): String? {
        framesUntil = maxOf(framesUntil, started + frames); requestsLeft = requests
        return takeSummary(nowMs)
    }

    /** Whether the frame numbered [number] (by start or by completion) is written. */
    fun frameWritten(number: Int): Boolean = number <= framesUntil

    /** A frame request; true when it is written, false when it is only counted. */
    fun request(granted: Boolean): Boolean {
        if (requestsLeft > 0) { requestsLeft--; return true }
        quietRequests++; if (granted) quietGranted++
        return false
    }

    /** A saved frame completed without being written. */
    fun quietFrame() { quietFrames++ }

    /** An ink frame was drawn; ink frames are only ever counted. */
    fun inkFrame() { inkFrames++ }

    /** Ink frames since the last written count, then zero. */
    fun takeInk(): Int = inkFrames.also { inkFrames = 0 }

    /** The counts, once at least [summaryMs] has passed since the last summary and
     * something was counted; null otherwise. */
    fun summaryDue(nowMs: Long): String? {
        val last = lastSummaryMs
        if (last != null && nowMs - last < summaryMs) return null
        return takeSummary(nowMs)
    }

    private fun takeSummary(nowMs: Long): String? {
        if (quietRequests == 0 && quietFrames == 0 && inkFrames == 0) return null
        val line = "quiet_requests=$quietRequests quiet_granted=$quietGranted quiet_frames=$quietFrames ink_frames=${takeInk()}"
        quietRequests = 0; quietGranted = 0; quietFrames = 0; lastSummaryMs = nowMs
        return line
    }
}
