package com.visualworkbench.desktop

import com.visualworkbench.shared.LocalSessionAddress
import com.visualworkbench.shared.PairedDevice
import com.visualworkbench.shared.ProjectSessionRole
import com.visualworkbench.shared.SessionCarrier
import com.visualworkbench.shared.SessionEndpoint
import com.visualworkbench.shared.checkedPeerLabel
import java.nio.file.Path

/** App-window memory only. No socket, trust operation, persistence or implicit
 * route selection belongs here. Pairing secrets and confirmation consent are
 * deliberately absent, so closing the pairing sheet still retires them. */
internal data class ConnectionDraft(
    val selectedAddress: LocalSessionAddress? = null,
    val bind: String = "",
    val method: PairingMethod = PairingMethod.Qr,
    val peer: ConnectionPeer? = null,
    val peerEndpoint: String = "",
    val local: EndpointFields = EndpointFields(),
    val remote: EndpointFields = EndpointFields(),
    val expectedProject: String = "",
    val phonePort: String = "47191",
    val hostPort: String = "47191",
    val copiedUsbSource: LocalSessionAddress? = null,
    val issues: Set<ConnectionDraftIssue> = emptySet(),
) {
    override fun toString(): String = "ConnectionDraft([REDACTED])"

    fun selectedPeer(paired: List<PairedDevice>): PairedDevice? {
        val selected = peer ?: return null
        return paired.firstOrNull { it.deviceId == selected.deviceId &&
            it.fingerprint == selected.fingerprint && it.revokedAtMs == null }
    }

    fun selectPeer(value: PairedDevice): ConnectionDraft = copy(
        peer = ConnectionPeer(value.deviceId, value.fingerprint),
        issues = issues - ConnectionDraftIssue.PeerUnavailable,
    )

    fun reconcilePeers(paired: List<PairedDevice>): ConnectionDraft =
        if (peer == null || selectedPeer(paired) != null) this
        else copy(peer = null, issues = issues + ConnectionDraftIssue.PeerUnavailable)

    fun selectedConnection(addresses: List<LocalSessionAddress>, refreshing: Boolean): LocalSessionAddress? =
        selectedAddress?.let { selected -> addresses.firstOrNull { !refreshing && it.sameConnection(selected) } }

    fun selectConnection(value: LocalSessionAddress): ConnectionDraft {
        val retireCopy = copiedUsbSource?.let { !it.sameConnection(value) } == true
        return copy(
            selectedAddress = value,
            bind = pairingEndpoint(value),
            local = if (retireCopy) local.copy(tether = "") else local,
            copiedUsbSource = if (retireCopy) null else copiedUsbSource,
            issues = (issues - ConnectionDraftIssue.ConnectionUnavailable).let {
                if (retireCopy) it + ConnectionDraftIssue.CopiedUsbRetired else it
            },
        )
    }

    fun reconcileConnections(addresses: List<LocalSessionAddress>): ConnectionDraft {
        val previous = selectedAddress ?: return this
        val fresh = addresses.firstOrNull { it.sameConnection(previous) }
        if (fresh != null) return copy(selectedAddress = fresh,
            copiedUsbSource = copiedUsbSource?.let { source -> addresses.firstOrNull { it.sameConnection(source) } })
        val retireCopy = copiedUsbSource != null
        return copy(
            selectedAddress = null,
            bind = if (bind == pairingEndpoint(previous)) "" else bind,
            local = if (retireCopy) local.copy(tether = "") else local,
            copiedUsbSource = null,
            issues = issues + ConnectionDraftIssue.ConnectionUnavailable +
                (if (retireCopy) setOf(ConnectionDraftIssue.CopiedUsbRetired) else emptySet()),
        )
    }

    /** An explicit click supplies this one carrier; it neither dials nor replaces
     * alternative/manual routes. A changed adapter needs another explicit click. */
    fun useSelectedUsb(addresses: List<LocalSessionAddress>, refreshing: Boolean): ConnectionDraft {
        val selected = selectedConnection(addresses, refreshing) ?: return this
        val endpoint = selectedUsbEndpoint(addresses, refreshing) ?: return this
        return copy(local = local.copy(tether = endpoint), copiedUsbSource = selected,
            issues = issues - ConnectionDraftIssue.CopiedUsbRetired)
    }

    /** Preserve an explicitly chosen port, but never attach an unrelated manual
     * pairing address to the selected adapter's USB safety check. */
    fun selectedUsbEndpoint(addresses: List<LocalSessionAddress>, refreshing: Boolean): String? {
        val selected = selectedConnection(addresses, refreshing) ?: return null
        val prefix = if (':' in selected.address) "[${selected.address}]:" else "${selected.address}:"
        val candidate = bind.trim()
        if (!candidate.startsWith(prefix)) return null
        val port = candidate.removePrefix(prefix)
        if (port.length !in 1..5 || port.any { it !in '0'..'9' }) return null
        val numericPort = port.toIntOrNull() ?: return null
        if (numericPort !in 1..65535) return null
        return candidate
    }

    fun manualLocalTether(value: String): ConnectionDraft = copy(
        local = local.copy(tether = value), copiedUsbSource = null,
        issues = issues - ConnectionDraftIssue.CopiedUsbRetired,
    )
}

internal data class ConnectionPeer(val deviceId: String, val fingerprint: String) {
    override fun toString(): String = "ConnectionPeer([REDACTED])"
}

internal enum class ConnectionDraftIssue { ConnectionUnavailable, CopiedUsbRetired, PeerUnavailable }

internal enum class PairingMethod(val label: String) {
    Qr("QR code"), Code("Eight-digit code"), Join("Enter code from another device")
}

/** Unselected carriers are absent; explicit alternatives keep their established
 * tether/Wi-Fi/ADB priority. This value is never logged or persisted. */
internal data class EndpointFields(val tether: String = "", val wifi: String = "", val adb: String = "") {
    fun endpoints(): List<SessionEndpoint> = listOf(SessionCarrier.QuicTether to tether,
        SessionCarrier.QuicWifi to wifi, SessionCarrier.TcpAdb to adb)
        .filter { it.second.isNotBlank() }.map { SessionEndpoint(it.first, it.second.trim()) }
    override fun toString(): String = "EndpointFields([REDACTED])"
}

internal enum class ProjectConnectionBlocker {
    PairingStorage, PairingActive, ProjectBusy, SelectPeer, OpenProject,
    ProjectRole, OriginalHost, RefreshingConnections, SelectConnection, PairingEndpointMismatch, ProjectEndpoint,
}

/** Pure readiness for an existing project's explicitly chosen connection.
 * Unknown ownership must never be treated as either host or replica. */
internal fun ConnectionDraft.projectBlocker(
    ready: Boolean, pairing: Boolean, busy: Boolean, hasProject: Boolean,
    role: ProjectSessionRole?, paired: List<PairedDevice>, addresses: List<LocalSessionAddress>,
    refreshing: Boolean, configureUsb: Boolean = false,
): ProjectConnectionBlocker? {
    connectionBlocker(ready, pairing, busy, paired)?.let { return it }
    if (!hasProject) return ProjectConnectionBlocker.OpenProject
    if (role == null) return ProjectConnectionBlocker.ProjectRole
    if (!role.isHost && selectedPeer(paired)?.deviceId != role.hostDeviceId)
        return ProjectConnectionBlocker.OriginalHost
    if (configureUsb) {
        addressBlocker(addresses, refreshing)?.let { return it }
        return if (selectedUsbEndpoint(addresses, refreshing) == null) ProjectConnectionBlocker.PairingEndpointMismatch else null
    }
    val endpoints = if (role.isHost) local.endpoints() else remote.endpoints()
    if (endpoints.isEmpty()) return ProjectConnectionBlocker.ProjectEndpoint
    return if (endpoints.any { it.carrier == SessionCarrier.QuicTether }) addressBlocker(addresses, refreshing) else null
}

/** Explicit advanced receive creates a new authenticated copy; it does not
 * guess the current project's role or alter its ownership. */
internal fun ConnectionDraft.receiveBlocker(
    ready: Boolean, pairing: Boolean, busy: Boolean, paired: List<PairedDevice>,
    addresses: List<LocalSessionAddress>, refreshing: Boolean,
): ProjectConnectionBlocker? {
    connectionBlocker(ready, pairing, busy, paired)?.let { return it }
    val endpoints = remote.endpoints()
    if (endpoints.isEmpty()) return ProjectConnectionBlocker.ProjectEndpoint
    return if (endpoints.any { it.carrier == SessionCarrier.QuicTether }) addressBlocker(addresses, refreshing) else null
}

private fun ConnectionDraft.connectionBlocker(
    ready: Boolean, pairing: Boolean, busy: Boolean, paired: List<PairedDevice>,
): ProjectConnectionBlocker? = when {
    !ready -> ProjectConnectionBlocker.PairingStorage
    pairing -> ProjectConnectionBlocker.PairingActive
    busy -> ProjectConnectionBlocker.ProjectBusy
    selectedPeer(paired) == null -> ProjectConnectionBlocker.SelectPeer
    else -> null
}

private fun ConnectionDraft.addressBlocker(addresses: List<LocalSessionAddress>, refreshing: Boolean): ProjectConnectionBlocker? = when {
    refreshing -> ProjectConnectionBlocker.RefreshingConnections
    selectedConnection(addresses, false) == null -> ProjectConnectionBlocker.SelectConnection
    else -> null
}

internal fun pairingEndpoint(address: LocalSessionAddress): String =
    if (':' in address.address) "[${address.address}]:44242" else "${address.address}:44242"

internal fun projectDirectoryFromPicker(chosen: Path): Path? =
    chosen.normalize().takeIf { it.fileName?.toString()?.equals("project.sqlite", ignoreCase = true) == true }?.parent

internal fun dialogAfterProjectPicker(returnToConnection: Boolean, current: Command?, quitting: Boolean): Command? =
    if (returnToConnection && current == null && !quitting) Command.Pairing else current

/** A presentation choice is separate from choosing a trusted peer or a route.
 * Showing a saved-phone path must never start pairing or auto-select a phone. */
internal enum class ConnectionScreenPath { Loading, PairNewPhone, PairedProject }

internal fun connectionScreenPath(ready: Boolean, paired: List<PairedDevice>,
    requestedNewPhone: Boolean?, pairing: Boolean): ConnectionScreenPath = when {
    !ready -> ConnectionScreenPath.Loading
    pairing -> ConnectionScreenPath.PairNewPhone
    (requestedNewPhone ?: paired.none { it.revokedAtMs == null }) -> ConnectionScreenPath.PairNewPhone
    else -> ConnectionScreenPath.PairedProject
}

internal enum class PairingActionBlocker { Storage, Active, Refreshing, LocalConnection, OtherEndpoint, Code }

/** No secrets are retained here. Join needs only the explicitly entered other
 * endpoint/code; showing a local offer waits for the local address refresh. */
internal fun ConnectionDraft.pairingBlocker(ready: Boolean, pairing: Boolean,
    refreshing: Boolean, enteredCode: String): PairingActionBlocker? = when {
    !ready -> PairingActionBlocker.Storage
    pairing -> PairingActionBlocker.Active
    method == PairingMethod.Join && peerEndpoint.isBlank() -> PairingActionBlocker.OtherEndpoint
    method == PairingMethod.Join && (enteredCode.length != 8 || enteredCode.any { it !in '0'..'9' }) -> PairingActionBlocker.Code
    method == PairingMethod.Join -> null
    refreshing -> PairingActionBlocker.Refreshing
    bind.isBlank() -> PairingActionBlocker.LocalConnection
    else -> null
}

/** Cosmetic local labels are never DNS, peer IDs or proof of USB reachability. */
internal fun computerLabel(name: String?): String = name?.let(::checkedPeerLabel) ?: "This Windows computer"

internal fun adapterLabel(address: LocalSessionAddress, addresses: List<LocalSessionAddress>): String {
    val interfaces = addresses.map { it.interfaceIndex }.distinct().sorted()
    val name = address.interfaceName?.let(::checkedPeerLabel)
    if (name == null) return "PC network connection ${interfaces.indexOf(address.interfaceIndex) + 1}"
    val matching = addresses.filter { it.interfaceName?.let(::checkedPeerLabel) == name }
        .map { it.interfaceIndex }.distinct().sorted()
    return if (matching.size > 1) "$name · connection ${matching.indexOf(address.interfaceIndex) + 1}" else name
}

internal fun addressOptionLabel(address: LocalSessionAddress, addresses: List<LocalSessionAddress>): String {
    val family = if (':' in address.address) "IPv6" else "IPv4"
    val routes = addresses.filter { it.interfaceIndex == address.interfaceIndex && (':' in it.address) == (':' in address.address) }
        .sortedBy { it.address }
    return if (routes.size > 1) "$family option ${routes.indexOfFirst { it.sameConnection(address) } + 1}" else family
}

internal fun networkConnectionLabel(address: LocalSessionAddress, addresses: List<LocalSessionAddress>): String =
    "${if (address.loopback) "Local developer · " else ""}${adapterLabel(address, addresses)} · ${addressOptionLabel(address, addresses)}"
