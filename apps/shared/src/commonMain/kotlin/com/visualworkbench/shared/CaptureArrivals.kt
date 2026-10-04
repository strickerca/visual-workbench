package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*

/** UI-dispatcher confined. The app serializes bind/detach with project ownership.
 * A notification is never a permission to replace an editor or discard drafts.
 * All producer cancellation is joined before the project may close. */
public class CaptureArrivals(
    private val scope: CoroutineScope,
    private val query: suspend (WorkbenchProject, ULong) -> ReceivedCapture? = { p, after -> p.receivedCapture(after) },
) {
    private data class Attachment(val project: WorkbenchProject, val epoch: Long, var after: ULong)
    private var attachment: Attachment? = null
    private var generation = 0L
    private var work: Job? = null
    private var requested = false
    private val mutable = MutableStateFlow<ReceivedCapture?>(null)
    public val state: StateFlow<ReceivedCapture?> = mutable.asStateFlow()
    private val mutableFailure = MutableStateFlow<String?>(null)
    public val failure: StateFlow<String?> = mutableFailure.asStateFlow()

    public fun bind(project: WorkbenchProject, epoch: Long, afterHostSeq: ULong): Unit {
        check(attachment == null && work == null)
        attachment = Attachment(project, epoch, afterHostSeq); generation++; mutable.value = null; mutableFailure.value = null
    }
    public fun changed(project: WorkbenchProject, epoch: Long): Unit {
        if (!scope.isActive) return
        val owner = attachment ?: return
        if (owner.project !== project || owner.epoch != epoch) return
        requested = true
        if (work != null) return
        val admitted = generation
        val next = scope.launch(start = CoroutineStart.LAZY) {
            while (requested && attachment === owner && generation == admitted) {
                requested = false
                try {
                    val value = query(owner.project, owner.after)
                    currentCoroutineContext().ensureActive()
                    if (attachment === owner && generation == admitted) { mutable.value = value; mutableFailure.value = null }
                } catch (cancel: CancellationException) { throw cancel }
                catch (_: Exception) {
                    if (attachment === owner && generation == admitted) {
                        mutableFailure.value = "A received capture could not be verified. Its saved project remains unchanged."
                    }
                }
            }
        }
        work = next
        next.invokeOnCompletion {
            if (work === next) {
                work = null
                if (scope.isActive && !next.isCancelled && requested && attachment === owner && generation == admitted) changed(owner.project, owner.epoch)
            }
        }
        next.start()
    }
    public fun acknowledge(value: ReceivedCapture): Unit {
        val owner = attachment ?: return
        if (mutable.value != value || !value.originalVerified) return
        owner.after = maxOf(owner.after, value.createdHostSeq); mutable.value = null
        changed(owner.project, owner.epoch)
    }
    public suspend fun detach(): Unit {
        generation++; attachment = null; requested = false; mutable.value = null
        val prior = work
        withContext(NonCancellable) { prior?.cancelAndJoin() }
        if (work === prior) work = null
        mutableFailure.value = null
    }
}
