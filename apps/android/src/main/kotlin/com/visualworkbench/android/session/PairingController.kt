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
    private var expiry: Job? = null
    var working by mutableStateOf(false); private set
    var peers by mutableStateOf<List<PairedDevice>>(emptyList()); private set
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
    var carrier by mutableStateOf(SessionCarrier.QuicWifi)
    var sessionAddress: String
        get() = when (carrier) { SessionCarrier.QuicTether -> tetherAddress; SessionCarrier.QuicWifi -> wifiAddress; SessionCarrier.TcpAdb -> adbAddress }
        set(value) { when (carrier) { SessionCarrier.QuicTether -> tetherAddress = value; SessionCarrier.QuicWifi -> wifiAddress = value; SessionCarrier.TcpAdb -> adbAddress = value } }
    var scanner by mutableStateOf(true)
    var problem by mutableStateOf<String?>(null); private set

    suspend fun service(): WorkbenchSessions = serviceLock.withLock {
        check(!closed)
        service?.let { return@withLock it }
        val created = factory(context.applicationContext, installationId())
        if (closed) { withContext(NonCancellable) { created.close() }; throw CancellationException() }
        service = created
        created
    }
    fun enter() { launch {
        val native = service(); local = native.localDevice(); peers = native.pairedDevices()
        localAddresses = localSessionAddresses()
    } }
    fun choosePeer(peer: String) { if (peers.any { it.deviceId == peer && it.revokedAtMs == null }) selectedPeer = peer }
    fun endpoint(): List<SessionEndpoint> {
        val configured = listOf(SessionCarrier.QuicTether to tetherAddress, SessionCarrier.QuicWifi to wifiAddress, SessionCarrier.TcpAdb to adbAddress).filter { it.second.isNotBlank() }
        require(configured.isNotEmpty() && configured.all { validEndpoint(it.second) }) { "Enter a numeric session address and port for each configured route." }
        return configured.map { SessionEndpoint(it.first, it.second.trim()) }
    }
    fun scan(bytes: ByteArray) {
        if (working || closed || bytes.size !in 1..4096) { bytes.fill(0); return }
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
    }
    fun pairQr() {
        val parsed = details ?: return
        if (pairingEndpoint !in parsed.endpoints) return
        launch {
            val owned = qr ?: throw SessionFailure(SessionFailureKind.Expired)
            try { paired(service().joinQr(owned, pairingEndpoint)) }
            finally { clearSecret() }
        }
    }
    fun pairCode() {
        if (code.length != 8 || code.any { it !in '0'..'9' } || !validEndpoint(pairingEndpoint)) {
            problem = "Enter the eight-digit code and the numeric pairing address shown on the computer."; return
        }
        val entered = code; code = ""
        launch {
            clearSecret()
            val token = secretEpoch
            val handle = service().joinCode(entered, pairingEndpoint.trim())
            if (token != secretEpoch || closed) { withContext(NonCancellable) { handle.close() }; return@launch }
            confirmation = handle; fingerprint = handle.fingerprint
            expiry = scope.launch { delay(300_000); if (token == secretEpoch) { expiry = null; clearSecret(); problem = "Confirmation expired. Request a new code." } }
        }
    }
    fun confirmFingerprint() {
        val handle = confirmation ?: return
        val displayed = fingerprint ?: return
        launch { try { paired(handle.confirm(displayed)) } finally { clearSecret() } }
    }
    fun decline() { launch { try { confirmation?.decline() } finally { clearSecret() } } }
    private suspend fun paired(peer: String) {
        peers = service().pairedDevices(); selectedPeer = peer
        notify("Computer paired. Choose its session address to open or receive a project.")
    }
    fun revoke(peer: String, before: suspend () -> Unit) = launch {
        before(); service().revoke(peer); peers = service().pairedDevices()
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
        secretEpoch++; operation?.cancel(); code = ""; scanner = false
        val pending = confirmation; confirmation = null
        qr?.fill(0); qr = null; details = null; fingerprint = null; expiry?.cancel(); expiry = null
        if (pending != null) scope.launch { withContext(NonCancellable) { runCatching { pending.close() } } }
    }
    private suspend fun clearSecret() {
        secretEpoch++; expiry?.cancel(); expiry = null
        qr?.fill(0); qr = null; details = null; fingerprint = null; code = ""
        val pending = confirmation; confirmation = null
        if (pending != null) withContext(NonCancellable) { pending.close() }
    }
    suspend fun close() {
        closed = true; leave(); operation?.join()
        serviceLock.withLock { service?.close(); service = null }
    }
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
