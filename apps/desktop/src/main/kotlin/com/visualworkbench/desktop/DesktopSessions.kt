package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Values in this state belong only to the visible pairing sheet. Never log or
 * persist the offer, code, endpoint or full device identity. */
class PairingDisplay internal constructor(val offer: PairingOffer, val qrText: String?) {
    override fun toString(): String = "PairingDisplay([REDACTED])"
}

data class DesktopSessionState(
    val ready: Boolean = false,
    val local: SessionDevice? = null,
    val paired: List<PairedDevice> = emptyList(),
    val pairing: Boolean = false,
    val display: PairingDisplay? = null,
    val fingerprint: String? = null,
    val confirming: Boolean = false,
    val discovered: List<DiscoveredPeer> = emptyList(),
    val discovering: Boolean = false,
    val message: String? = null,
) {
    override fun toString(): String = "DesktopSessionState(ready=$ready,pairing=$pairing)"
}

/** App lifetime, one pairing attempt, and one bounded trust action at a time.
 * Native acquisitions already settle cancellation and release undelivered
 * handles. Keep them on this coroutine; another dispatcher handoff could lose
 * the returned owned value at a cancellation boundary. */
class DesktopSessionController(
    parent: CoroutineScope,
    private val installationDevice: String,
    private val factory: suspend (String) -> WorkbenchSessions = ::createDesktopSessions,
    private val clock: () -> Long = System::currentTimeMillis,
) {
    private val lifetime = SupervisorJob(parent.coroutineContext[Job])
    private val scope = CoroutineScope(parent.coroutineContext + lifetime)
    private val mutable = MutableStateFlow(DesktopSessionState())
    val state: StateFlow<DesktopSessionState> = mutable
    private val closeMutex = Mutex()
    private var closed = false
    private var service: WorkbenchSessions? = null
    private var pairingJob: Job? = null
    private var trustJob: Job? = null
    private var discoveryJob: Job? = null
    private var confirmationDecision: CompletableDeferred<Boolean>? = null
    private var confirmationText: String? = null
    private val initialized = scope.async {
        val created = factory(installationDevice)
        var adopted = false
        try {
            val local = created.localDevice()
            require(local.deviceId == installationDevice) { "Installed device identity mismatch" }
            val paired = created.pairedDevices()
            currentCoroutineContext().ensureActive()
            service = created
            adopted = true
            mutable.value = mutable.value.copy(ready = true, local = local, paired = paired)
            created
        } finally {
            if (!adopted) withContext(NonCancellable) { created.close() }
        }
    }.also { work ->
        work.invokeOnCompletion { error ->
            if (error != null && error !is CancellationException)
                mutable.value = mutable.value.copy(message = "Pairing storage could not open. The installed device identity was preserved.")
        }
    }

    suspend fun requireService(): WorkbenchSessions {
        if (closed) throw SessionFailure(SessionFailureKind.Closed)
        return initialized.await()
    }

    fun clearMessage() { mutable.value = mutable.value.copy(message = null) }

    fun browse(localAddresses: List<String>) {
        if (closed || discoveryJob?.isActive == true) return
        val selected = localAddresses.toList()
        mutable.value = mutable.value.copy(discovering = true, discovered = emptyList())
        discoveryJob = scope.launch {
            var discovery: SessionDiscovery? = null
            try {
                discovery = requireService().browseDiscovery(selected)
                while (currentCoroutineContext().isActive) {
                    mutable.value = mutable.value.copy(discovered = discovery.snapshot())
                    delay(1000)
                }
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { mutable.value = mutable.value.copy(message = sessionMessage(error)) }
            finally {
                withContext(NonCancellable) { discovery?.close() }
                mutable.value = mutable.value.copy(discovering = false, discovered = emptyList())
            }
        }
    }
    fun stopDiscovery() { discoveryJob?.cancel() }

    fun refreshTrust() = trustAction { sessions ->
        mutable.value = mutable.value.copy(paired = sessions.pairedDevices(), message = null)
    }

    fun revoke(deviceId: String, disconnect: suspend (String) -> Unit) = trustAction { sessions ->
        // Durable revocation is checked by existing native links as well. Stop
        // their UI collectors explicitly before forgetting their display state.
        sessions.revoke(deviceId)
        disconnect(deviceId)
        mutable.value = mutable.value.copy(paired = sessions.pairedDevices(), message = "Pairing revoked. This device must pair again.")
    }

    private fun trustAction(block: suspend (WorkbenchSessions) -> Unit) {
        if (closed || trustJob?.isActive == true) return
        trustJob = scope.launch {
            try { block(requireService()) }
            catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { mutable.value = mutable.value.copy(message = sessionMessage(error)) }
        }
    }

    fun offer(endpoint: String, useCode: Boolean) = beginPairing {
        val sessions = requireService()
        val server = sessions.listenPairing(checkedEndpoint(endpoint))
        var displayed: PairingOffer? = null
        var advertised: SessionDiscovery? = null
        try {
            displayed = server.offer(useCode)
            val remaining = displayed.expiresAtMs.toLong() - clock()
            if (remaining !in 1..300_000) throw SessionFailure(SessionFailureKind.Expired)
            mutable.value = mutable.value.copy(display = PairingDisplay(displayed,
                if (useCode) null else encodePairingQr(displayed.qr)), message = null)
            try { advertised = sessions.createDiscovery(listOf(server.endpoint)) }
            catch (_: SessionFailure) { mutable.value = mutable.value.copy(message = "The pairing listener is open; nearby discovery is unavailable. Use its displayed address.") }
            withTimeout(remaining) {
                when (val result = server.accept()) {
                    is PairingResult.Paired -> paired(sessions, result.deviceId)
                    is PairingResult.Confirm -> confirm(sessions, result.confirmation)
                }
            }
        } finally {
            // A server confirmation shares the listener runtime. Release the
            // confirmation first (inside confirm), then close the listener.
            mutable.value.display?.offer?.clear()
            displayed?.clear()
            mutable.value = mutable.value.copy(display = null)
            withContext(NonCancellable) { try { advertised?.close() } finally { server.close() } }
        }
    }

    fun joinCode(code: String, endpoint: String) {
        if (!code.matches(Regex("[0-9]{8}"))) {
            mutable.value = mutable.value.copy(message = "Enter the eight-digit code shown on the other device.")
            return
        }
        beginPairing {
            val sessions = requireService()
            withTimeout(300_000) { confirm(sessions, sessions.joinCode(code, checkedEndpoint(endpoint))) }
        }
    }

    private suspend fun confirm(sessions: WorkbenchSessions, handle: PairingConfirmation) {
        try {
            val full = handle.fingerprint
            val decision = CompletableDeferred<Boolean>()
            confirmationText = full
            confirmationDecision = decision
            mutable.value.display?.offer?.clear()
            mutable.value = mutable.value.copy(display = null, fingerprint = full, confirming = false)
            if (decision.await()) {
                mutable.value = mutable.value.copy(confirming = true)
                paired(sessions, handle.confirm(full))
            } else handle.decline()
        } finally {
            confirmationDecision = null
            confirmationText = null
            mutable.value = mutable.value.copy(fingerprint = null, confirming = false)
            withContext(NonCancellable) { handle.close() }
        }
    }

    /** Caller supplies the exact displayed value only after a deliberate user
     * confirmation. A stale dialog can never approve the next attempt. */
    fun confirmFingerprint(displayed: String, userComparedBothDevices: Boolean) {
        if (userComparedBothDevices && displayed == confirmationText)
            confirmationDecision?.complete(true)
    }

    fun decline() { confirmationDecision?.complete(false) }
    fun cancelPairing() {
        mutable.value.display?.offer?.clear()
        mutable.value = mutable.value.copy(display = null, fingerprint = null)
        pairingJob?.cancel()
    }

    private suspend fun paired(sessions: WorkbenchSessions, device: String) {
        // The returned ID is intentionally not included in user-facing logs.
        require(device.isNotBlank())
        mutable.value = mutable.value.copy(paired = sessions.pairedDevices(), message = "Device paired. Select it to share or receive a project.")
    }

    private fun beginPairing(block: suspend () -> Unit) {
        if (closed || pairingJob?.isActive == true) return
        mutable.value = mutable.value.copy(pairing = true, message = null)
        pairingJob = scope.launch {
            try { block() }
            catch (_: TimeoutCancellationException) { mutable.value = mutable.value.copy(message = "Pairing expired. Create a new offer to try again.") }
            catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { mutable.value = mutable.value.copy(message = sessionMessage(error)) }
            finally {
                confirmationDecision = null; confirmationText = null
                mutable.value.display?.offer?.clear()
                mutable.value = mutable.value.copy(pairing = false, display = null, fingerprint = null, confirming = false)
            }
        }
    }

    suspend fun close() = withContext(NonCancellable) {
        closeMutex.withLock {
            if (closed) return@withLock
            closed = true
            cancelPairing()
            pairingJob?.cancelAndJoin()
            trustJob?.cancelAndJoin()
            discoveryJob?.cancelAndJoin()
            initialized.cancelAndJoin()
            try { service?.close() } finally {
                service = null
                mutable.value = DesktopSessionState()
                lifetime.cancel()
            }
        }
    }
}

/** Do not resolve DNS or silently broaden a listener. The native parser is the
 * final authority for numeric IP, scope, port, multicast and wildcard checks. */
internal fun checkedEndpoint(value: String): String {
    val text = value.trim()
    require(text.length in 3..128 && text.none { it.isWhitespace() || it.isISOControl() })
    return text
}

internal fun sessionMessage(error: Exception): String = when (error) {
    is SessionFailure -> when (error.kind) {
        SessionFailureKind.LockedOut -> "Pairing is locked after failed attempts. Wait ten minutes before trying again."
        SessionFailureKind.Authentication -> "Authentication failed. Check pairing and the full fingerprint on both devices."
        SessionFailureKind.Expired -> "Pairing expired. Create a new offer."
        SessionFailureKind.Declined -> "Pairing was declined."
        SessionFailureKind.Backpressure -> "The connection is busy. Wait for the current operation to finish."
        SessionFailureKind.Storage -> "Secure pairing storage could not be updated. Existing identity and project files were preserved."
        SessionFailureKind.Timeout -> "The peer did not respond in time. Check the selected connection and try again."
        SessionFailureKind.Invalid -> "Check the selected device and numeric endpoint, including its port."
        else -> "The session could not continue. Check the connection and pairing, then retry."
    }
    is IllegalArgumentException -> "Enter a numeric endpoint with a port, such as the address shown on the other device."
    else -> "The session could not continue. Existing projects and pairing identity were preserved."
}

internal fun syncLabel(status: SessionStatus?, pending:UInt=0u): String = when (status?.status) {
    SyncStatus.Synced -> "SYNCED"
    SyncStatus.Syncing -> "SYNCING"
    SyncStatus.Reconnecting -> "RECONNECTING"
    SyncStatus.Offline, null -> "OFFLINE (${status?.pending ?: pending} pending)"
}
