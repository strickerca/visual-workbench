package com.visualworkbench.android.editor

import kotlinx.coroutines.CompletableDeferred
import java.util.concurrent.atomic.AtomicInteger

/** Main-confined renderer-generation bookkeeping for one canvas surface.
 *
 * A generation is one AndroidX renderer with its buffer pool and scene slot. At
 * most two are alive: a presented generation kept during a make-before-break
 * replacement, and its successor. A generation is allocated only when none is
 * current and no release is settling, so a third pool can never be allocated
 * while an older one is still being released. */
internal class CanvasGenerations {
    private val releases = LinkedHashSet<CompletableDeferred<Unit>>()
    private var last: CompletableDeferred<Unit>? = null
    val releasing: Boolean get() = releases.isNotEmpty()
    val releaseCount: Int get() = releases.size

    /** A fresh generation needs no current one and no release still settling. */
    fun canCreate(hasCurrent: Boolean): Boolean = !hasCurrent && releases.isEmpty()

    /** A replacement keeps the current generation presented while exactly one
     * successor is created. Giant mode owns a single surface reservation and
     * never replaces in place. */
    fun canReplace(hasCurrent: Boolean, hasKept: Boolean, giant: Boolean): Boolean =
        hasCurrent && !hasKept && !giant && releases.isEmpty()

    /** Registers an actual renderer release; the token completes on Main once the
     * release callback has closed the generation's reader. */
    fun beginRelease(): CompletableDeferred<Unit> = CompletableDeferred<Unit>().also { releases += it; last = it }

    /** Main: the token's completion (successful or not) has been published. */
    fun settled(token: CompletableDeferred<Unit>) { releases.remove(token) }

    /** Completes after every release begun so far and fails if any of them failed.
     * With nothing outstanding it is the most recent release, matching the single
     * retirement the surface exposed before replacements could overlap. */
    fun joined(): CompletableDeferred<Unit> {
        val pending = releases.toList()
        if (pending.isEmpty()) return last ?: CompletableDeferred(Unit)
        if (pending.size == 1) return pending.single()
        val result = CompletableDeferred<Unit>()
        val remaining = AtomicInteger(pending.size)
        for (token in pending) token.invokeOnCompletion { error ->
            if (error != null) result.completeExceptionally(error)
            else if (remaining.decrementAndGet() == 0) result.complete(Unit)
        }
        return result
    }
}

/** SurfaceView cuts its window hole once every redraw callback has finished. A
 * canvas with no committed frame would then expose SurfaceView's opaque black
 * background, so finishing waits for this canvas's first committed frame, bounded
 * by a fallback so a gated successor can never stall a window sync indefinitely. */
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
        pending += drawingFinished
        unschedule(fallback); schedule(fallback, fallbackMs)
    }

    fun finish() {
        unschedule(fallback)
        if (pending.isEmpty()) return
        val ready = pending.toList(); pending.clear()
        for (callback in ready) callback.run()
    }
}
