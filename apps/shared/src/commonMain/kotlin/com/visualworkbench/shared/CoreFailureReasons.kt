package com.visualworkbench.shared

/** Turns a core failure the app was just handed into plain words: what
 * happened and what to do. The core records why it failed as fixed wording;
 * a reason is used only once and only if it is newer than the last one used,
 * so an old failure never explains a new one. The reason itself is never
 * shown; it only selects a more specific message in [ErrorCatalog].
 *
 * [read] returns the core's events after the given sequence number, or null
 * when they cannot be read (the general message for the failure kind is then
 * used). If two threads ask at the same moment the worst outcome is that
 * one reason is used twice. */
public class CoreFailureReasons(private val read: (afterSequence: Long) -> CoreEventLog?) {
    @kotlin.concurrent.Volatile private var lastSequence = 0L

    public fun advice(kind: CoreFailureKind): ProblemAdvice {
        val fresh = read(lastSequence)
        val newest = fresh?.events?.lastOrNull()?.sequence
        if (newest != null && newest > lastSequence) lastSequence = newest
        return ErrorCatalog.core(kind, fresh?.latestFailure()?.name)
    }

    public fun text(error: CoreFailure): String = advice(error.kind).let { "${it.whatHappened} ${it.whatToDo}" }
}
