package com.visualworkbench.android.editor

internal enum class EditorCloseStage {
    Requested, Queued, Navigation, CaptureStream, Assisted, MutationGate, MutationAcquired,
    ProjectClose, ImageFrames, Workflows, Rotation, DocumentOwner, Giant, Erasers, Selections,
    InstructionFocus, Semantics, Instructions, Ai, ChangeObserver, RefreshObserver, Link,
    ProjectStorage, ProjectsCatalog, NavigationInstructionCleanup, Finished,
}

internal enum class EditorCloseStatus { Running, Settled, Failed, Completed, Declined }

internal data class EditorCloseSnapshot(
    val attempt: Int,
    val stage: EditorCloseStage,
    val status: EditorCloseStatus,
    val failedStage: EditorCloseStage? = null,
)

/** Main-confined diagnostic state only. No job, timeout, resource or admission
 * is owned here. A queued caller cannot replace an active caller's last phase.
 * One latest snapshot is retained per actual unfinished attempt, never completed
 * history. Sequence numbers are bounded labels; ownership uses Attempt identity. */
internal class EditorCloseProgress(private val observe: (EditorCloseSnapshot) -> Unit = {}) {
    var current: EditorCloseSnapshot? = null
        private set
    private var sequence = 0
    private var active: Attempt? = null
    private val live = linkedMapOf<Attempt, EditorCloseSnapshot>()

    fun begin(): Attempt {
        sequence = if (sequence == Int.MAX_VALUE) 1 else sequence + 1
        val attempt = Attempt(sequence)
        live[attempt] = EditorCloseSnapshot(sequence, EditorCloseStage.Queued, EditorCloseStatus.Running)
        if (active == null) attempt.update(EditorCloseStage.Requested, EditorCloseStatus.Running)
        else emit(EditorCloseSnapshot(sequence, EditorCloseStage.Queued, EditorCloseStatus.Running))
        return attempt
    }

    private fun emit(snapshot: EditorCloseSnapshot) {
        // A diagnostic observer must not change an operation's result/cleanup.
        try { observe(snapshot) } catch (_: Exception) { }
    }

    inner class Attempt internal constructor(private val number: Int) {
        private var failed: EditorCloseStage? = null
        private var latest: EditorCloseSnapshot? = null
        private var finished = false

        internal fun update(stage: EditorCloseStage, status: EditorCloseStatus) {
            if (finished) return
            val snapshot = EditorCloseSnapshot(number, stage, status, failed)
            latest = snapshot
            live[this] = snapshot
            if (active == null) active = this
            if (active === this) { current = snapshot; emit(snapshot) }
        }

        fun reached(stage: EditorCloseStage) { update(stage, EditorCloseStatus.Settled) }

        suspend fun <T> step(stage: EditorCloseStage, operation: suspend () -> T): T {
            update(stage, EditorCloseStatus.Running)
            try {
                val value = operation()
                update(stage, EditorCloseStatus.Settled)
                return value
            } catch (error: Throwable) {
                if (failed == null) {
                    failed = stage
                    update(stage, EditorCloseStatus.Failed)
                } else if (latest?.stage == stage && latest?.status == EditorCloseStatus.Running) {
                    // Cleanup can fail too. Preserve the first failed join and
                    // do not relabel a propagated inner error as its wrapper.
                    update(stage, EditorCloseStatus.Failed)
                }
                throw error
            }
        }

        fun finish(completed: Boolean = true) {
            if (finished) return
            val failure = failed
            if (failure != null) update(failure, EditorCloseStatus.Failed)
            else update(EditorCloseStage.Finished,
                if (completed) EditorCloseStatus.Completed else EditorCloseStatus.Declined)
            finished = true
            live.remove(this)
            if (active === this) {
                active = live.keys.firstOrNull()
                // A successor may already be suspended after leaving the edit
                // gate. Publish its last real phase now, without resuming it.
                active?.let { live[it] }?.let { snapshot -> current = snapshot; emit(snapshot) }
            }
        }
    }
}
