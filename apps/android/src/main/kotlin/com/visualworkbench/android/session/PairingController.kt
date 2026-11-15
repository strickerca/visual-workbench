package com.visualworkbench.android.session

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** One Keystore service for the existing installation identity; no project owns keys. */
internal class PairingController(
    private val context: Context,
    private val scope: CoroutineScope,
    private val installationId: () -> String,
    private val notify: (String) -> Unit,
    private val factory: suspend (Context, String) -> WorkbenchSessions = ::createAndroidSessions,
) {
    private val serviceLock = Mutex()
    private var service: WorkbenchSessions? = null
    private var closed = false
    private var operation: Job? = null
    private var secretEpoch = 0L
    private var qr: ByteArray? = null
    private var confirmation: PairingConfirmation? = null
    private var confirmationEndpoint: String? = null
    private var expiry: Job? = null
    private val peerLabels by lazy { PeerLabelStore(context.noBackupFilesDir.canonicalFile.toPath().resolve("peer-labels.properties")) }
    private var labels by mutableStateOf<PeerLabelSnapshot?>(null)
    private var namingOperation: Job? = null
    private var connectionHint by mutableStateOf<PairingConnectionHint?>(null)
    var namingWorking by mutableStateOf(false); private set
    var namingProblem by mutableStateOf<String?>(null); private set
    val namesWritable: Boolean get() = labels?.writable == true
    var working by mutableStateOf(false); private set
    var peers by mutableStateOf<List<PairedDevice>>(emptyList()); private set
    var peersLoaded by mutableStateOf(false); private set
    var pairingCompletion by mutableStateOf(0L); private set
    val scanRequestEpoch: Long get() = secretEpoch
    var selectedPeer by mutableStateOf<String?>(null); private set
    var local by mutableStateOf<SessionDevice?>(null); private set
    var localAddresses by mutableStateOf<List<LocalSessionAddress>>(emptyList()); private set
    var details by mutableStateOf<QrDetails?>(null); private set
    var fingerprint by mutableStateOf<String?>(null); private set
    var pairingEndpoint by mutableStateOf("")
    var code by mutableStateOf("")
    private var tetherAddress by mutableStateOf("")
    private var wifiAddress by mutableStateOf("")
    private var adbAddress by mutableStateOf("")
    var carrier by mutableStateOf(SessionCarrier.QuicTether)
    var sessionAddress: String
        get() = when (carrier) { SessionCarrier.QuicTether -> tetherAddress; SessionCarrier.QuicWifi -> wifiAddress; SessionCarrier.TcpAdb -> adbAddress }
        set(value) { when (carrier) { SessionCarrier.QuicTether -> tetherAddress = value; SessionCarrier.QuicWifi -> wifiAddress = value; SessionCarrier.TcpAdb -> adbAddress = value } }
    var scanner by mutableStateOf(false)
    var problem by mutableStateOf<String?>(null); private set

    // Pair once, then no typing. What is remembered is app-level, beside the
    // peer names: never in a project, a log or a report.
    private val routeDiscovery by lazy { AndroidRouteDiscovery(context.applicationContext) }
    private val automaticPairing by lazy {
        AutomaticPairing(AutomaticRouteKeeper(
            KnownRouteStore(context.noBackupFilesDir.canonicalFile.toPath().resolve("known-routes.properties")), routeDiscovery,
            { _ -> { address -> isAndroidUsbTetherInterface(address.interfaceName) } },
            // Wi-Fi and the phone's own USB tethering only; never mobile data.
            { _ -> { address -> isAndroidWifiInterface(address.interfaceName) || isAndroidUsbTetherInterface(address.interfaceName) } }),
            routeDiscovery)
    }
    private var realService: WorkbenchSessions? = null
    private var automaticOperation: Job? = null
    /** The owner's setting. While on and no address is typed, connecting needs no address. */
    var automatic by mutableStateOf(AUTOMATIC_BY_DEFAULT); private set
    /** It is staying out of the way after the owner ended the connection. */
    var automaticQuiet by mutableStateOf(false); private set

    /** The editor's service. Its connect, host and receive calls take automatic
     * routes when they are given the endpoints from [endpoint] in automatic mode. */
    suspend fun service(): WorkbenchSessions = serviceLock.withLock {
        check(!closed)
        service?.let { return@withLock it }
        val created = factory(context.applicationContext, installationId())
        if (closed) { withContext(NonCancellable) { created.close() }; throw CancellationException() }
        realService = created
        automaticPairing.routed(created).also { service = it }
    }
    private suspend fun nativeService(): WorkbenchSessions { service(); return checkNotNull(realService) }
    private fun manualAddresses(): Boolean = tetherAddress.isNotBlank() || wifiAddress.isNotBlank() || adbAddress.isNotBlank()

    /** One step of automatic connection, called while the app is in the
     * foreground. True when the caller should now connect the open project to
     * [selectedPeer]. It never interrupts pairing or the owner's own actions. */
    suspend fun automaticStep(project: () -> String?, status: () -> SessionStatus?, busy: () -> Boolean): Boolean {
        if (closed) return false
        // Switched off: nothing is opened, announced, looked for, dialled or saved.
        if (!automaticPairing.load()) { automatic = false; automaticQuiet = false; return false }
        automatic = true
        // Nothing to connect and the protected pairing store not yet opened:
        // leave it closed until a project or the Connect screen needs it.
        if (realService == null && project() == null) return false
        val native = nativeService()
        if (!peersLoaded) {
            val paired = native.pairedDevices()
            if (!peersLoaded && !closed) { updatePeers(paired); peersLoaded = true }
        }
        val target = automaticPairing.step(native, { peers }) { AutomaticView(project(), status(), busy() || working, manualAddresses()) }
        automatic = automaticPairing.enabled; automaticQuiet = automaticPairing.quiet
        if (target == null || closed) return false
        // The same choice the owner would make; the editor still checks that
        // the project belongs to this computer.
        if (peers.none { it.deviceId == target && it.revokedAtMs == null }) return false
        selectedPeer = target
        return true
    }
    fun chooseAutomatic(value: Boolean) {
        if (closed || automaticOperation?.isActive == true) return
        automaticOperation = scope.launch {
            try { automaticPairing.setEnabled(value); automatic = automaticPairing.enabled; automaticQuiet = automaticPairing.quiet }
            catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { /* The setting stays as it was. */ }
        }
    }
    fun automaticProjectClosed() { automaticPairing.projectClosed(); automaticQuiet = false }
    /** The editor was busy and did not start the connection [automaticStep] asked for. */
    fun automaticMissed() = automaticPairing.missed()
    /** The app left the foreground: stop announcing and looking. */
    suspend fun automaticRest() { if (!closed) automaticPairing.rest() }
    fun enter() { launch {
        val token = secretEpoch
        val native = service()
        val paired = native.pairedDevices()
        currentCoroutineContext().ensureActive()
        if (token != secretEpoch || closed) return@launch
        updatePeers(paired); peersLoaded = true
        loadNames(token)
        // Optional address enumeration cannot turn loaded trust into unknown-empty.
        val device = native.localDevice()
        currentCoroutineContext().ensureActive()
        if (token != secretEpoch || closed) return@launch
        local = device
        val addresses = localSessionAddresses()
        currentCoroutineContext().ensureActive()
        if (token == secretEpoch && !closed) localAddresses = addresses
    } }
    fun choosePeer(peer: String) {
        if (peers.any { it.deviceId == peer && it.revokedAtMs == null }) {
            if (selectedPeer != peer) connectionHint = null
            selectedPeer = peer
        }
    }
    fun activePeers(): List<PairedDevice> = peers.filter { it.revokedAtMs == null }
        .sortedWith(compareBy<PairedDevice> { it.pairedAtMs }.thenBy { it.deviceId })
    fun peerName(peer: PairedDevice): String = labels?.nameFor(peer.deviceId, peer.fingerprint)
        ?: "Computer ${activePeers().indexOfFirst { it.deviceId == peer.deviceId }.let { if (it < 0) 1 else it + 1 }}"
    fun peerDisplayName(peer: PairedDevice): String {
        val name = peerName(peer)
        val duplicates = activePeers().filter { peerName(it).equals(name, ignoreCase = true) }
        if (duplicates.size < 2) return name
        val suffix = peer.fingerprint.take(12)
        return if (duplicates.count { it.fingerprint.take(12) == suffix } == 1) "$name · $suffix"
        else "$name · Computer ${activePeers().indexOfFirst { it.deviceId == peer.deviceId } + 1}"
    }
    private suspend fun loadNames(token: Long) {
        val loaded = try { withContext(Dispatchers.IO) { peerLabels.load() } }
        catch (cancel: CancellationException) { throw cancel }
        catch (_: Exception) { null }
        currentCoroutineContext().ensureActive()
        if (token != secretEpoch || closed) return
        labels = loaded
        namingProblem = if (loaded == null) "Computer names could not be loaded. Pairing and connections remain available."
        else labelMessage(loaded.issue)
    }
    fun renamePeer(peerId: String, expectedFingerprint: String, value: String) {
        if (working || namingWorking || closed) return
        val peer = activePeers().singleOrNull { it.deviceId == peerId && it.fingerprint == expectedFingerprint }
        if (peer == null || selectedPeer != peerId) { namingProblem = "This computer changed. Choose it again before renaming."; return }
        val checked = checkedPeerLabel(value)
        if (checked == null) { namingProblem = "Use a single-line name with 1–64 characters, without control or direction characters."; return }
        if (!namesWritable) { namingProblem = "Saved computer names are unavailable. Existing naming data is preserved."; return }
        val token = secretEpoch
        namingWorking = true; namingProblem = null
        namingOperation = scope.launch {
            try {
                val saved = withContext(Dispatchers.IO) { peerLabels.rename(peer.deviceId, peer.fingerprint, checked) }
                currentCoroutineContext().ensureActive()
                if (token == secretEpoch && !closed) { labels = saved; namingProblem = labelMessage(saved.issue) }
            } catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { if (token == secretEpoch && !closed) namingProblem = "The computer name could not be saved. Pairing and connections remain available." }
            finally { namingWorking = false }
        }
    }
    private fun updatePeers(paired: List<PairedDevice>) {
        peers = paired
        if (currentHint() == null) connectionHint = null
        // With exactly one paired computer there is nothing to choose.
        if (automatic && selectedPeer == null) activePeers().singleOrNull()?.let { selectedPeer = it.deviceId }
    }
    private fun currentHint(): PairingConnectionHint? {
        val hint = connectionHint ?: return null
        return hint.takeIf { it.peerDeviceId == selectedPeer && validEndpoint(it.endpoint) &&
            peers.count { peer -> peer.revokedAtMs == null && peer.deviceId == it.peerDeviceId && peer.fingerprint == it.fingerprint } == 1 }
    }
    fun canUsePairingConnection(): Boolean = !working && !closed && currentHint() != null
    /** Explicit UI action; updates only the currently chosen route, never trust or a live link. */
    fun usePairingConnection() {
        if (working || closed) return
        val hint = currentHint() ?: return
        sessionAddress = hint.endpoint
    }
    fun configuredCarriers(): List<SessionCarrier> =
        listOf(SessionCarrier.QuicTether to tetherAddress, SessionCarrier.QuicWifi to wifiAddress, SessionCarrier.TcpAdb to adbAddress)
            .filter { it.second.isNotBlank() }.map { it.first }
    fun endpoint(): List<SessionEndpoint> {
        val configured = listOf(SessionCarrier.QuicTether to tetherAddress, SessionCarrier.QuicWifi to wifiAddress, SessionCarrier.TcpAdb to adbAddress).filter { it.second.isNotBlank() }
        // No typed address and automatic connection on: the routed service
        // finds the computer itself. A typed address always means manual.
        if (configured.isEmpty() && automatic) return automaticPairing.endpoints
        require(configured.isNotEmpty() && configured.all { validEndpoint(it.second) }) { "Enter a numeric session address and port for each configured route." }
        return configured.map { SessionEndpoint(it.first, it.second.trim()) }
    }
    fun scan(bytes: ByteArray): Boolean {
        if (working || closed || bytes.size !in 1..4096) { bytes.fill(0); return false }
        connectionHint = null
        scanner = false
        val owned = bytes.copyOf(); bytes.fill(0)
        launch {
            try {
                clearSecret()
                currentCoroutineContext().ensureActive()
                qr = owned
                val token = secretEpoch
                val parsed = service().inspectQr(owned)
                if (token != secretEpoch || closed) { owned.fill(0); return@launch }
                val now = System.currentTimeMillis().toULong()
                check(parsed.expiresAtMs > now)
                details = parsed; pairingEndpoint = parsed.endpoints.firstOrNull().orEmpty()
                expiry = scope.launch {
                    delay((parsed.expiresAtMs - now).coerceAtMost(300_000uL).toLong())
                    if (token == secretEpoch) { expiry = null; clearSecret(); problem = "This QR code expired. Scan a new code on the computer." }
                }
            } catch (error: Exception) { clearSecret(); throw error }
            finally { if (qr !== owned) owned.fill(0) }
        }
        return true
    }
    fun pairQr() {
        val parsed = details ?: return
        if (pairingEndpoint !in parsed.endpoints) return
        val selectedEndpoint = pairingEndpoint
        launch {
            val owned = qr ?: throw SessionFailure(SessionFailureKind.Expired)
            try { paired(service().joinQr(owned, selectedEndpoint), selectedEndpoint, parsed.fingerprint, parsed.peerDeviceId) }
            finally { clearSecret() }
        }
    }
    fun pairCode() {
        if (code.length != 8 || code.any { it !in '0'..'9' } || !validEndpoint(pairingEndpoint)) {
            problem = "Enter the eight-digit code and the numeric pairing address shown on the computer."; return
        }
        val entered = code; code = ""; connectionHint = null
        val selectedEndpoint = pairingEndpoint.trim()
        launch {
            clearSecret()
            val token = secretEpoch
            val handle = service().joinCode(entered, selectedEndpoint)
            if (token != secretEpoch || closed) { withContext(NonCancellable) { handle.close() }; return@launch }
            confirmation = handle; confirmationEndpoint = selectedEndpoint; fingerprint = handle.fingerprint
            expiry = scope.launch { delay(300_000); if (token == secretEpoch) { expiry = null; clearSecret(); problem = "Confirmation expired. Request a new code." } }
        }
    }
    fun confirmFingerprint() {
        val handle = confirmation ?: return
        val displayed = fingerprint ?: return
        val selectedEndpoint = confirmationEndpoint ?: return
        launch { try { paired(handle.confirm(displayed), selectedEndpoint, displayed) } finally { clearSecret() } }
    }
    fun decline() { launch { try { confirmation?.decline() } finally { clearSecret() } } }
    private suspend fun paired(peer: String, selectedEndpoint: String, expectedFingerprint: String, expectedPeer: String? = null) {
        val paired = service().pairedDevices()
        currentCoroutineContext().ensureActive()
        updatePeers(paired); peersLoaded = true; selectedPeer = peer
        val authenticated = paired.singleOrNull { it.deviceId == peer && it.revokedAtMs == null && it.fingerprint == expectedFingerprint }
        connectionHint = authenticated?.takeIf { (expectedPeer == null || it.deviceId == expectedPeer) && validEndpoint(selectedEndpoint) }
            ?.let { PairingConnectionHint(it.deviceId, it.fingerprint, selectedEndpoint) }
        // Pairing just authenticated the computer at this address; its project
        // listener uses the same one. Kept as the first hint, never as trust.
        val hinted = connectionHint
        if (authenticated != null && hinted != null) {
            try { automaticPairing.learned(nativeService(), authenticated, hinted.endpoint) }
            catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { /* Discovery can still find the computer. */ }
        }
        loadNames(secretEpoch); pairingCompletion++
        notify(if (automatic && !manualAddresses()) "Computer paired. Open a project on the computer, then tap Receive computer project here. No address is needed."
            else "Computer paired. On the PC, start its project connection; then use the pairing connection here or enter its session address.")
    }
    fun revoke(peer: String, before: suspend () -> Unit) = launch {
        before(); service().revoke(peer); updatePeers(service().pairedDevices())
        if (connectionHint?.peerDeviceId == peer) connectionHint = null
        if (selectedPeer == peer) selectedPeer = null
    }
    private fun launch(action: suspend () -> Unit) {
        if (working || closed) return
        working = true; problem = null
        operation = scope.launch {
            try { action() }
            catch (cancel: CancellationException) { throw cancel }
            catch (error: SessionFailure) { problem = sessionMessage(error.kind) }
            catch (_: Exception) { problem = "Pairing could not finish safely. Check the computer and try again." }
            finally { working = false }
        }
    }
    /** Called on screen exit/background: no QR, code or pending confirmation survives. */
    fun leave() {
        secretEpoch++; operation?.cancel(); namingOperation?.cancel(); connectionHint = null; code = ""; scanner = false
        confirmationEndpoint = null
        val pending = confirmation; confirmation = null
        qr?.fill(0); qr = null; details = null; fingerprint = null; expiry?.cancel(); expiry = null
        if (pending != null) scope.launch { withContext(NonCancellable) { runCatching { pending.close() } } }
    }
    private suspend fun clearSecret() {
        secretEpoch++; expiry?.cancel(); expiry = null
        qr?.fill(0); qr = null; details = null; fingerprint = null; code = ""; confirmationEndpoint = null
        val pending = confirmation; confirmation = null
        if (pending != null) withContext(NonCancellable) { pending.close() }
    }
    suspend fun close() {
        closed = true; leave(); operation?.join(); namingOperation?.join(); automaticOperation?.cancelAndJoin()
        withContext(NonCancellable) { runCatching { automaticPairing.rest() } }
        serviceLock.withLock { service?.close(); service = null; realService = null }
    }
}

/** Numeric public hint only; never contains QR credentials and is never persisted. */
private class PairingConnectionHint(val peerDeviceId: String, val fingerprint: String, val endpoint: String)

private fun labelMessage(issue: PeerLabelIssue?): String? = when (issue) {
    null -> null
    PeerLabelIssue.InvalidName -> "Use a single-line name with 1–64 characters, without control or direction characters."
    PeerLabelIssue.InvalidPeer -> "This computer changed. Choose it again before renaming."
    PeerLabelIssue.SaveFailed -> "The computer name could not be saved. Pairing and connections remain available."
    PeerLabelIssue.Unreadable, PeerLabelIssue.InvalidStore, PeerLabelIssue.ChangedStore ->
        "Saved computer names are unavailable. Existing naming data is preserved; pairing and connections remain available."
}

internal fun validEndpoint(value: String): Boolean {
    val text = value.trim()
    if (text.length !in 3..128) return false
    val split = text.lastIndexOf(':'); if (split <= 0) return false
    val port = text.substring(split + 1).toIntOrNull() ?: return false
    if (port !in 1..65535) return false
    val host = text.substring(0, split)
    return if (host.startsWith('[') && host.endsWith(']')) {
        val address = host.substring(1, host.length - 1)
        address.contains(':') && address.all { it in "0123456789abcdefABCDEF:.%" || it in '0'..'9' }
    } else host.split('.').let { it.size == 4 && it.all { part -> part.isNotEmpty() && part.all(Char::isDigit) && (part.toIntOrNull() ?: -1) in 0..255 } }
}

internal fun sessionMessage(kind: SessionFailureKind): String = when (kind) {
    SessionFailureKind.Authentication -> "The computer identity could not be verified. Check its fingerprint and pairing state."
    SessionFailureKind.Expired -> "This pairing code expired. Request a new code."
    SessionFailureKind.LockedOut -> "Pairing is temporarily locked after failed attempts. Wait ten minutes before trying again."
    SessionFailureKind.Declined -> "Fingerprint confirmation was declined. No new trust was granted."
    SessionFailureKind.Timeout, SessionFailureKind.Transport -> "The computer is unavailable. Check Wi-Fi or USB tethering and its displayed address."
    SessionFailureKind.Storage -> "The protected pairing store could not be updated. Saved projects are unchanged."
    SessionFailureKind.Backpressure -> "The connection is at its resource limit. Try again after the current operation finishes."
    else -> "The connection could not complete (${kind.name}). Saved work is intact."
}
