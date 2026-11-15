package com.visualworkbench.android.editor

import kotlinx.coroutines.CompletableDeferred

/** Strokes are saved in the order their pens went down, whatever order their
 * ink finishes in. Each stroke takes a turn at pen-down, waits for the turn
 * before it when it is ready to save, and gives its own turn up as soon as its
 * save was attempted (saved, refused or cancelled) so the next can go.
 * Confined to the thread that owns the strokes. */
internal class StrokeTurns {
    private var newest: CompletableDeferred<Unit>? = null

    class Turn internal constructor(private val earlier: CompletableDeferred<Unit>?, private val own: CompletableDeferred<Unit>) {
        /** Suspends until the stroke begun before this one has had its save attempted. */
        suspend fun awaitEarlier() { earlier?.await() }
        /** Idempotent. Must run on every exit, or later strokes never save. */
        fun done() { own.complete(Unit) }
    }

    fun enter(): Turn {
        val own = CompletableDeferred<Unit>()
        return Turn(newest, own).also { newest = own }
    }
}
