package com.visualworkbench.android.editor

import kotlinx.coroutines.CompletableDeferred

/** Main-confined release bookkeeping for one canvas surface's renderer generations.
 *
 * A generation is one AndroidX renderer with its buffer pools and scene slot. No
 * generation is allocated while any release is still settling. Releases that
 * overlap in time form one batch; [joined] completes only after every release of
 * the batch has settled and then fails with the batch's first failure. */
internal class CanvasGenerations {
    private val releases = LinkedHashSet<CompletableDeferred<Unit>>()
    private var batch: CompletableDeferred<Unit>? = null
    private var batchFailure: Throwable? = null
    val releasing: Boolean get() = releases.isNotEmpty()
    val releaseCount: Int get() = releases.size

    /** A fresh generation needs no current one and no release still settling. */
    fun canCreate(hasCurrent: Boolean): Boolean = !hasCurrent && releases.isEmpty()

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
) {
    private val pending = ArrayList<Runnable>()
    private val fallback = Runnable { finish() }
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
