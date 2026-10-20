package com.visualworkbench.android.editor

import java.util.concurrent.atomic.AtomicReference

internal enum class DocumentRetirementStage {
    Requested, TaskJoin, DrawGate, DrawGateAcquired, Adopt, SurfaceRetirement,
    FrameRetirement, CacheClose, Finished,
}

internal enum class DocumentRetirementStatus { Running, Settled, Failed, Completed }

internal data class DocumentRetirementSnapshot(
    val attempt: Int,
    val stage: DocumentRetirementStage,
    val status: DocumentRetirementStatus,
    val failedStage: DocumentRetirementStage? = null,
)

/** Main-confined observation only. Retain one snapshot per live attempt so a
 * concurrent detach cannot hide the first join or a suspended successor. */
internal class DocumentRetirementProgress {
    var current: DocumentRetirementSnapshot? = null
        private set
    private var sequence = 0
    private var active: Attempt? = null
    private val live = linkedMapOf<Attempt, DocumentRetirementSnapshot>()

    fun begin(): Attempt {
        sequence = nextRetirementSequence(sequence)
        return Attempt(sequence).also { it.update(DocumentRetirementStage.Requested, DocumentRetirementStatus.Running) }
    }

    inner class Attempt internal constructor(private val number: Int) {
        private var failed: DocumentRetirementStage? = null
        private var finished = false

        internal fun update(stage: DocumentRetirementStage, status: DocumentRetirementStatus) {
            if (finished) return
            val snapshot = DocumentRetirementSnapshot(number, stage, status, failed)
            live[this] = snapshot
            if (active == null) active = this
            if (active === this) current = snapshot
        }

        fun reached(stage: DocumentRetirementStage) { update(stage, DocumentRetirementStatus.Settled) }

        suspend fun <T> step(stage: DocumentRetirementStage, operation: suspend () -> T): T {
            update(stage, DocumentRetirementStatus.Running)
            try {
                val value = operation()
                update(stage, DocumentRetirementStatus.Settled)
                return value
            } catch (error: Throwable) {
                if (failed == null) {
                    failed = stage
                    update(stage, DocumentRetirementStatus.Failed)
                }
                throw error
            }
        }

        fun finish() {
            if (finished) return
            val failure = failed
            if (failure == null) update(DocumentRetirementStage.Finished, DocumentRetirementStatus.Completed)
            else update(failure, DocumentRetirementStatus.Failed)
            finished = true
            live.remove(this)
            if (active === this) {
                active = live.keys.firstOrNull()
                active?.let { live[it] }?.let { current = it }
            }
        }
    }
}

internal enum class CanvasRetirementReason {
    SurfaceChanged, SurfaceDestroyed, WindowDetached, DocumentFrameChanged, ProjectRetirement,
}

internal enum class CanvasRetirementMilestone {
    ReleaseRequested, LibraryCallbackEntered, MainCallbackEntered, ReaderCloseRunning,
    ReaderClosed, CompletionPublished, Failed,
}

internal data class CanvasRetirementSnapshot(
    val generation: Int,
    val reason: CanvasRetirementReason,
    val milestone: CanvasRetirementMilestone,
    val failedMilestone: CanvasRetirementMilestone? = null,
)

/** A release token is created only for an actual renderer release. Callback
 * observations are visible before Main delivery; old tokens never replace a
 * newer release, even when the bounded diagnostic number wraps. */
internal class CanvasRetirementProgress {
    private var sequence = 0 // begin is Main-confined; callbacks only update their token.
    private val active = AtomicReference<Release?>(null)
    val current: CanvasRetirementSnapshot? get() = active.get()?.snapshot?.get()

    fun begin(reason: CanvasRetirementReason): Release {
        sequence = nextRetirementSequence(sequence)
        return Release(CanvasRetirementSnapshot(sequence, reason, CanvasRetirementMilestone.ReleaseRequested))
            .also { active.set(it) }
    }

    inner class Release internal constructor(initial: CanvasRetirementSnapshot) {
        internal val snapshot = AtomicReference(initial)

        fun reached(milestone: CanvasRetirementMilestone) {
            update { old ->
                if (old.failedMilestone != null || milestone.ordinal <= old.milestone.ordinal) old
                else old.copy(milestone = milestone)
            }
        }

        fun failed(milestone: CanvasRetirementMilestone) {
            update { old -> if (old.failedMilestone != null) old else old.copy(
                milestone = CanvasRetirementMilestone.Failed, failedMilestone = milestone) }
        }

        private fun update(change: (CanvasRetirementSnapshot) -> CanvasRetirementSnapshot) {
            while (active.get() === this) {
                val old = snapshot.get()
                val next = change(old)
                if (next === old || snapshot.compareAndSet(old, next)) return
            }
        }
    }
}

private fun nextRetirementSequence(previous: Int): Int = if (previous == Int.MAX_VALUE) 1 else previous + 1

/** Fixed one-line handoff to the finite reporter. Only bounded diagnostic
 * counters and enum names are serialized; the legacy close fields stay last. */
internal fun formatRetirementTimeoutSuffix(
    doc: DocumentRetirementSnapshot?,
    canvas: CanvasRetirementSnapshot?,
    close: EditorCloseSnapshot?,
): String =
    "doc_attempt=${doc?.attempt ?: 0} doc_stage=${doc?.stage?.name ?: "None"} " +
    "doc_status=${doc?.status?.name ?: "None"} doc_failed_stage=${doc?.failedStage?.name ?: "None"} " +
    "canvas_generation=${canvas?.generation ?: 0} canvas_reason=${canvas?.reason?.name ?: "None"} " +
    "canvas_milestone=${canvas?.milestone?.name ?: "None"} canvas_failed_milestone=${canvas?.failedMilestone?.name ?: "None"} " +
    "close_attempt=${close?.attempt ?: 0} close_stage=${close?.stage?.name ?: "None"} " +
    "close_status=${close?.status?.name ?: "None"} close_failed_stage=${close?.failedStage?.name ?: "None"}"
