package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

public data class MarkerFocusState(
    public val enabled: Boolean = false, public val available: Boolean = false,
    public val latest: PeerMarkerFocus? = null, public val message: String? = null,
    public val connectionEpoch: ULong = 0uL,
)
/** Owner UI dispatcher only. The callback selects an instruction; it is never a
 * camera callback. Existing text is retained whenever canFollow is false. App
 * owners call revisionChanged after publishing an exact document revision, and
 * detach before closing the borrowed link/project. Attach always disables follow.
 * This controller never sends a message, preventing remote-selection echoes. */
public class MarkerFocusFollower(
    private val scope: CoroutineScope,
    private val binding: () -> WorkflowBinding?,
    private val canFollow: () -> Boolean,
    private val select: (PeerMarkerFocus) -> Unit,
) {
    private val mutable: MutableStateFlow<MarkerFocusState> = MutableStateFlow(MarkerFocusState())
    public val state: StateFlow<MarkerFocusState> = mutable
    private var capability: WorkbenchMarkerFocus? = null
    private var collector: Job? = null
    private var worker: Job? = null
    private var queue: Channel<Unit>? = null
    private var generation: ULong = 0uL
    private var ticket: ULong = 0uL
    private var signal: FocusSignal = FocusSignal(0uL, 0uL, false)
    private var applied: Triple<ULong, ULong, WorkflowBinding>? = null
    private val ownership: Mutex = Mutex()
    private var invocation: ULong = 0uL

    public suspend fun attach(value: WorkbenchMarkerFocus): Unit {
        val entered = ++invocation
        ownership.withLock {
            retire()
            currentCoroutineContext().ensureActive()
            if (entered != invocation) return@withLock
            capability = value
            val owned = generation
            val requests = Channel<Unit>(Channel.CONFLATED)
            queue = requests
            worker = scope.launch {
                while (requests.receiveCatching().isSuccess) {
                    if (owned != generation || capability !== value || !mutable.value.enabled || !signal.available) continue
                    val expected = binding() ?: continue
                    val observed = signal; val requested = ticket
                    try {
                        val focus = value.peer(expected)
                        if (owned != generation || capability !== value || requested != ticket ||
                            binding() != expected || observed != signal || value.signal() != observed ||
                            !mutable.value.enabled || !signal.available) continue
                        val key = focus?.let { Triple(it.connectionEpoch, it.sequence, it.binding) }
                        if (focus != null && (focus.binding != expected || focus.connectionEpoch != signal.connectionEpoch)) continue
                        mutable.value = mutable.value.copy(latest = focus, message = null)
                        if (focus != null && key != applied) {
                            if (!canFollow()) {
                                mutable.value = mutable.value.copy(message = "Save or discard the current instruction draft before following peer focus.")
                            } else { select(focus); applied = key }
                        }
                    } catch (cancel: CancellationException) { throw cancel }
                    catch (_: Exception) {
                        if (owned == generation && requested == ticket) mutable.value = mutable.value.copy(latest = null,
                            message = "Peer focus is unavailable on this revision. Refresh after synchronization.")
                    }
                }
            }
            collector = scope.launch {
                try { value.signals.collect { next ->
                    if (owned != generation || capability !== value || next.sequence < signal.sequence) return@collect
                    val changed = signal.connectionEpoch != next.connectionEpoch || signal.available != next.available
                    signal = next; ticket++
                    if (changed) applied = null
                    mutable.value = mutable.value.copy(available = next.available, latest = null, connectionEpoch = next.connectionEpoch)
                    requests.trySend(Unit)
                } } catch (cancel: CancellationException) { throw cancel }
                catch (_: Exception) {
                    if (owned == generation) {
                        signal = signal.copy(available = false); ticket++; applied = null
                        mutable.value = mutable.value.copy(available = false, latest = null, message = "Reconnect to receive peer focus.")
                    }
                }
            }
            }
    }
    public fun enabled(value: Boolean): Unit {
        ticket++; applied = null
        mutable.value = mutable.value.copy(enabled = value, latest = null, message = null)
        if (value) queue?.trySend(Unit)
    }
    /** Also call after explicitly resolving a dirty draft if follow is desired. */
    public fun revisionChanged(): Unit {
        ticket++; applied = null
        mutable.value = mutable.value.copy(latest = null)
        queue?.trySend(Unit)
    }
    public suspend fun detach(): Unit {
        ++invocation
        // A later detach must also join a producer already being retired by a
        // pending attach before the owner may close the borrowed native link.
        withContext(NonCancellable) { ownership.withLock { retire() } }
    }
    private suspend fun retire(): Unit {
        generation++; ticket++; capability = null; signal = FocusSignal(0uL, 0uL, false); applied = null
        val priorCollector = collector; collector = null
        val priorWorker = worker; worker = null
        queue?.close(); queue = null
        mutable.value = MarkerFocusState()
        withContext(NonCancellable) { priorCollector?.cancelAndJoin(); priorWorker?.cancelAndJoin() }
    }
}
