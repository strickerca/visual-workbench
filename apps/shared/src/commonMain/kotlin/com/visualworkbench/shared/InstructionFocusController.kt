package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

public data class InstructionFocusState(
    public val available: Boolean = false, public val sharing: Boolean = false,
    public val following: Boolean = false, public val peerMarkerId: String? = null,
    public val message: String? = null,
)
/** Actual app ownership adapter. All calls use the owner UI dispatcher. It
 * borrows both editor and link, joins all producers before detach returns, and
 * owns no camera. Apps publish their exact document first, then revisionChanged.
 * Sharing/following are session choices; disconnect/reconnect resets both. */
public class InstructionFocusController(
    private val scope: CoroutineScope,
    private val editor: InstructionEditor,
    private val attachment: () -> InstructionAttachment?,
    private val maySelect: () -> Boolean,
    private val selectMarker: (String, Boolean) -> Unit,
    private val capability: (ProjectLink) -> WorkbenchMarkerFocus = ::markerFocus,
    private val selectedObject: () -> String? = { null },
) {
    private val mutable: MutableStateFlow<InstructionFocusState> = MutableStateFlow(InstructionFocusState())
    public val state: StateFlow<InstructionFocusState> = mutable
    private var current: WorkbenchMarkerFocus? = null
    private var generation: ULong = 0uL
    private var localTicket: ULong = 0uL
    private var peerTicket: ULong = 0uL
    private var suppressedPeer: FocusSignal? = null
    private var shareEpoch: ULong? = null
    private var followEpoch: ULong? = null
    private var localIntent: Boolean = false
    private var localMarker: String? = null
    private var pending: Channel<Unit>? = null
    private var publisher: Job? = null
    private var sendAttempt: Deferred<Unit>? = null
    private var followWatch: Job? = null
    private var fieldWatch: Job? = null
    private val ownership: Mutex = Mutex()
    private var invocation: ULong = 0uL
    private val follower: MarkerFocusFollower = MarkerFocusFollower(scope,
        { attachment()?.binding },
        { maySelect() && !editor.state.value.busy && !editor.hasUnsaved() },
        ::acceptPeer)

    public suspend fun attach(link: ProjectLink): Unit {
        val enteredInvocation = ++invocation
        ownership.withLock {
            retire()
            currentCoroutineContext().ensureActive()
            if (enteredInvocation != invocation) return@withLock
            val value = capability(link)
            val owned = generation
            current = value
            val requests = Channel<Unit>(Channel.CONFLATED); pending = requests
            try {
                follower.attach(value)
                if (enteredInvocation != invocation) { retire(); return@withLock }
                publisher = scope.launch {
                    while (requests.receiveCatching().isSuccess) {
                        // Coalesce rapid native field/selection/revision callbacks
                        // before admitting another canonical hash/query on the core.
                        delay(50)
                        while (requests.tryReceive().isSuccess) { /* latest state below */ }
                        if (owned != generation || current !== value || !mutable.value.sharing || !localIntent) continue
                        val ticket = localTicket
                        try { supervisorScope {
                            val context = attachment() ?: return@supervisorScope
                            val marker = localMarker?.takeIf { id -> context.document.render.items.any { it.objectId == id && it.shape is Shape.Marker } }
                            val entered = value.signal()
                            if (!entered.available || entered.connectionEpoch != shareEpoch) return@supervisorScope
                            val attempt = async(start = CoroutineStart.LAZY) {
                                if (owned == generation && current === value && ticket == localTicket &&
                                    mutable.value.sharing && context.binding == attachment()?.binding && value.signal() == entered) {
                                    value.send(context.binding, marker)
                                }
                            }
                            sendAttempt = attempt; attempt.start()
                            try { attempt.await() }
                            finally { if (sendAttempt === attempt) sendAttempt = null }
                        } }
                        catch (cancel: CancellationException) { currentCoroutineContext().ensureActive() }
                        catch (_: Exception) {
                            if (owned == generation && ticket == localTicket) mutable.value = mutable.value.copy(
                                message = "Marker selection could not be shared on this revision. Select it again after synchronization.")
                        }
                    }
                }
                followWatch = scope.launch {
                    var wasAvailable = false
                    var priorEpoch = 0uL
                    follower.state.collect { follow ->
                        if (owned != generation || current !== value) return@collect
                        // StateFlow may coalesce the brief unavailable state; the
                        // carrier epoch still forces fresh explicit choices.
                        val lost = (wasAvailable && !follow.available) ||
                            (priorEpoch != 0uL && priorEpoch != follow.connectionEpoch)
                        if (lost) {
                            sharing(false); following(false)
                        }
                        wasAvailable = follow.available
                        priorEpoch = follow.connectionEpoch
                        mutable.value = mutable.value.copy(available = follow.available,
                            following = if (lost) false else follow.enabled, peerMarkerId = follow.latest?.markerId,
                            message = follow.message ?: mutable.value.message)
                    }
                }
                fieldWatch = scope.launch {
                    var wasBlocked = editor.state.value.busy || editor.hasUnsaved()
                    editor.state.collect { field ->
                        if (owned != generation) return@collect
                        val blocked = field.busy || field.field?.dirty == true
                        if (wasBlocked && !blocked) follower.revisionChanged()
                        wasBlocked = blocked
                    }
                }
            } catch (error: Throwable) { retire(); throw error }
        }
    }
    public fun sharing(enabled: Boolean): Unit {
        if (enabled && (!mutable.value.available || current == null)) return
        val now = if (enabled) try { current?.signal()?.takeIf { it.available } } catch (_: Exception) { null } else null
        if (enabled && now == null) return
        shareEpoch = now?.connectionEpoch
        localTicket++; sendAttempt?.cancel()
        mutable.value = mutable.value.copy(sharing = enabled, message = null)
        if (enabled) {
            if (!localIntent) localMarker = selectedObject()?.takeIf { id ->
                attachment()?.document?.render?.items?.any { it.objectId == id && it.shape is Shape.Marker } == true
            }
            localIntent = true
            pending?.trySend(Unit)
        }
    }
    public fun following(enabled: Boolean): Unit {
        if (enabled && (!mutable.value.available || current == null)) return
        val now = if (enabled) try { current?.signal()?.takeIf { it.available } } catch (_: Exception) { null } else null
        if (enabled && now == null) return
        followEpoch = now?.connectionEpoch
        peerTicket++
        if (enabled) suppressedPeer = null
        follower.enabled(enabled)
    }
    /** Invoke only for an actual local canvas selection. Incoming selection
     * updates app state directly and never call this method. */
    public fun localSelection(objectId: String?): Unit {
        peerTicket++; localTicket++; sendAttempt?.cancel(); localIntent = true
        // Local intent wins over an already-observed hint, including one whose
        // field acquisition is still settling. A newer peer hint may follow.
        suppressedPeer = try { current?.signal() } catch (_: Exception) { null }
        localMarker = objectId?.takeIf { id -> attachment()?.document?.render?.items?.any { it.objectId == id && it.shape is Shape.Marker } == true }
        mutable.value = mutable.value.copy(message = null)
        if (mutable.value.sharing) pending?.trySend(Unit)
    }
    /** Called by the editor only after a locally requested field is published;
     * includes a newly committed marker. Peer field publication never calls it. */
    public fun localField(field: InstructionField): Unit {
        if (field.binding != attachment()?.binding) return
        val marker = markerFor(field)
        marker?.let { selectMarker(it, false) }
        localSelection(marker)
    }
    private fun markerFor(field: InstructionField): String? {
        val context = attachment() ?: return null
        return field.targetIds.singleOrNull()?.takeIf { id -> context.document.render.items.any { it.objectId == id && it.shape is Shape.Marker } }
    }
    public fun revisionChanged(): Unit {
        peerTicket++; localTicket++; sendAttempt?.cancel()
        follower.revisionChanged()
        if (mutable.value.sharing && localIntent) pending?.trySend(Unit)
    }
    /** A modal/gesture admission change invalidates an already acquiring field.
     * Requery on dismissal preserves the current draft and never announces it. */
    public fun interactionChanged(): Unit { peerTicket++; follower.revisionChanged() }
    private fun acceptPeer(focus: PeerMarkerFocus) {
        val value = current ?: return
        val observed = try { value.signal() } catch (_: Exception) { return }
        if (!observed.available || observed.connectionEpoch != focus.connectionEpoch ||
            observed.connectionEpoch != followEpoch ||
            observed == suppressedPeer ||
            attachment()?.binding != focus.binding || !maySelect() || editor.hasUnsaved() || editor.state.value.busy) return
        val owned = generation; val request = ++peerTicket
        // Retire only the local announcement producer, never canonical edits.
        localIntent = false; localTicket++; sendAttempt?.cancel()
        fun valid(): Boolean = owned == generation && current === value && request == peerTicket &&
            follower.state.value.enabled && observed.connectionEpoch == followEpoch && attachment()?.binding == focus.binding && !editor.hasUnsaved() &&
            maySelect() && try { value.signal() == observed } catch (_: Exception) { false }
        editor.followInstruction(focus.instructionId, focus.binding, ::valid) {
            if (valid()) selectMarker(focus.markerId, true)
        }
    }
    public suspend fun detach(): Unit {
        ++invocation
        withContext(NonCancellable) { ownership.withLock { retire() } }
    }
    private suspend fun retire(): Unit {
        generation++; localTicket++; peerTicket++; current = null
        localIntent = false; localMarker = null; suppressedPeer = null; shareEpoch = null; followEpoch = null
        pending?.close(); pending = null
        val publish = publisher; publisher = null
        val watch = followWatch; followWatch = null
        val fields = fieldWatch; fieldWatch = null
        sendAttempt?.cancel()
        mutable.value = InstructionFocusState()
        withContext(NonCancellable) {
            try { watch?.cancelAndJoin(); fields?.cancelAndJoin(); follower.detach() }
            finally { publish?.cancelAndJoin(); sendAttempt = null }
        }
    }
}
