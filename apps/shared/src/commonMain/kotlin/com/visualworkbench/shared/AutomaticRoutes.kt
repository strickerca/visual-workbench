package com.visualworkbench.shared

/** The port every automatic project listener uses. It is the pairing port as
 * well, so the address in a scanned pairing code also leads to the project. */
public const val AUTOMATIC_SESSION_PORT: Int = 44242

/** Whether automatic connection is on before the owner has chosen. It stays
 * off, and the app then behaves exactly as it did without it, until automatic
 * connection has been run on the real phone and computer. */
public const val AUTOMATIC_BY_DEFAULT: Boolean = false

private const val MAX_ROUTE_HINTS: Int = 16
private const val MAX_AUTOMATIC_LISTENERS: Int = 6

/** Android names its own USB tethering interfaces rndis<n>, usb<n> or ncm<n>.
 * Only the phone that provides the tether may use this to recognise it. */
public fun isAndroidUsbTetherInterface(name: String?): Boolean {
    val text = name ?: return false
    val prefix = listOf("rndis", "usb", "ncm").firstOrNull { text.startsWith(it) } ?: return false
    val number = text.substring(prefix.length)
    return number.length in 1..2 && number.all { it in '0'..'9' }
}

/** Android names its Wi-Fi interfaces wlan<n> (and swlan<n>, softap<n> or
 * ap_br_wlan<n> while the phone is a hotspot). Mobile data is never one. */
public fun isAndroidWifiInterface(name: String?): Boolean {
    val text = name ?: return false
    val prefix = listOf("ap_br_wlan", "softap", "swlan", "wlan").firstOrNull { text.startsWith(it) } ?: return false
    val number = text.substring(prefix.length)
    return number.length in 1..2 && number.all { it in '0'..'9' }
}

private fun ipv4(text: String): Int? {
    val parts = text.split('.')
    if (parts.size != 4) return null
    var value = 0
    for (part in parts) {
        if (part.length !in 1..3 || part.any { it !in '0'..'9' }) return null
        val number = part.toInt()
        if (number > 255) return null
        value = (value shl 8) or number
    }
    return value
}

private fun sameSubnet(first: Int, second: Int, prefix: Int): Boolean =
    prefix in 1..32 && (first xor second) ushr (32 - prefix) == 0

private class NumericEndpoint(val host: String, val port: Int, val v4: Int?)

/** Syntax only. The native core decides which addresses may be dialled. */
private fun numericEndpoint(text: String): NumericEndpoint? {
    if (text.length !in 3..128 || text.any { it.isWhitespace() }) return null
    val split = text.lastIndexOf(':')
    if (split <= 0) return null
    val portText = text.substring(split + 1)
    if (portText.length !in 1..5 || portText.any { it !in '0'..'9' }) return null
    val port = portText.toInt()
    if (port !in 1..65535) return null
    val host = text.substring(0, split)
    if (host.startsWith('[') && host.endsWith(']')) {
        val inner = host.substring(1, host.length - 1)
        val valid = inner.count { it == ':' } >= 2 &&
            inner.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' || it == ':' || it == '.' || it == '%' }
        return if (valid) NumericEndpoint(inner, port, null) else null
    }
    val v4 = ipv4(host) ?: return null
    return NumericEndpoint(host, port, v4)
}

/** True for `a.b.c.d:port` and `[ipv6]:port`. Names are never accepted. */
public fun isNumericSessionEndpoint(text: String): Boolean = numericEndpoint(text) != null

/** Label untrusted address hints (from discovery or a scanned pairing code)
 * with the carrier this device would use to reach them. An address inside the
 * subnet of a local USB tethering link is USB tethering; everything else is
 * Wi-Fi. The label only orders the attempts and gates what needs USB: whether
 * the paired device is really there is decided by its pinned certificate. */
public fun classifyRouteHints(
    addresses: List<String>,
    local: List<LocalSessionAddress>,
    tether: (LocalSessionAddress) -> Boolean,
): List<SessionEndpoint> {
    val own = local.map { it.address.substringBefore('%') }.toSet()
    val links = local.filter { !it.loopback }.mapNotNull { address ->
        val bits = ipv4(address.address) ?: return@mapNotNull null
        val prefix = address.prefixLength ?: return@mapNotNull null
        Triple(bits, prefix, tether(address))
    }
    val seen = HashSet<String>()
    val result = ArrayList<SessionEndpoint>()
    for (raw in addresses) {
        val text = raw.trim()
        val endpoint = numericEndpoint(text) ?: continue
        if (endpoint.host.substringBefore('%') in own || endpoint.host == "::1") continue
        val bits = endpoint.v4
        if (bits != null && bits ushr 24 == 127) continue
        if (!seen.add(text)) continue
        val usb = bits != null && links.any { (address, prefix, usbLink) -> usbLink && sameSubnet(address, bits, prefix) }
        result.add(SessionEndpoint(if (usb) SessionCarrier.QuicTether else SessionCarrier.QuicWifi, text))
    }
    return result.sortedBy { it.carrier.ordinal }.take(MAX_ROUTE_HINTS)
}

/** The addresses an automatic listener binds, out of the [local] addresses
 * the caller allows it to listen on: IPv4 on real links only, USB tethering
 * first. No wildcard and no loopback is ever included. The caller passes only
 * Wi-Fi and USB tethering links, never every interface the device has. */
public fun automaticBindEndpoints(
    local: List<LocalSessionAddress>,
    tether: (LocalSessionAddress) -> Boolean,
    port: Int = AUTOMATIC_SESSION_PORT,
): List<SessionEndpoint> {
    require(port in 1..65535)
    return local.asSequence()
        .filter { !it.loopback }
        .filter { address -> ipv4(address.address)?.let { it ushr 24 != 127 && it ushr 16 != 0xA9FE } == true }
        .distinctBy { it.address }
        .map { Triple(if (tether(it)) SessionCarrier.QuicTether else SessionCarrier.QuicWifi, it.interfaceIndex, it.address) }
        .sortedWith(compareBy({ it.first.ordinal }, { it.second }, { it.third }))
        .take(MAX_AUTOMATIC_LISTENERS)
        .map { SessionEndpoint(it.first, "${it.third}:$port") }
        .toList()
}

/** The paired device an automatic connection is for, or null when the choice
 * is not obvious and the owner must make it. A replica connects only to the
 * device that hosts its project. A host with exactly one paired device uses
 * it; with several, the one used last. Revoked devices are never chosen. */
public fun automaticPeer(
    paired: List<PairedDevice>,
    role: ProjectSessionRole?,
    usedLast: (PairedDevice) -> Boolean,
): PairedDevice? {
    val active = paired.filter { it.revokedAtMs == null }
    if (role != null && !role.isHost) return active.singleOrNull { it.deviceId == role.hostDeviceId }
    return active.singleOrNull() ?: active.singleOrNull(usedLast)
}

/** What the app knows when it asks whether to start a connection by itself. */
public data class AutomaticSituation(
    /** The owner's setting. */
    public val enabled: Boolean,
    /** Identity of the open project instance; null when no project is open. */
    public val project: String?,
    /** The paired device chosen by [automaticPeer]; null when there is none. */
    public val peer: String?,
    /** A project connection exists, in whatever state. */
    public val linked: Boolean,
    /** The app is in the middle of another operation. */
    public val busy: Boolean,
    /** Any text that changes when the local network addresses change. */
    public val network: String,
) {
    override fun toString(): String = "AutomaticSituation(enabled=$enabled,linked=$linked,busy=$busy)"
}

/** Decides when the app may start a project connection without being asked.
 * It starts at most once for each open project, paired device and network
 * state, and never works against the owner: after a connection has gone away
 * (the owner disconnected, connected by hand, or revoked the device) it stays
 * quiet for that project until [resume] or until the project is opened again.
 * Not thread-safe; call it from the UI thread. */
public class AutomaticConnection {
    private var tried: Triple<String, String, String>? = null
    private var quietFor: String? = null
    private var linkedIn: String? = null

    /** The paired device to connect to now, or null to do nothing. */
    public fun next(situation: AutomaticSituation): String? {
        val project = situation.project
        if (project == null) {
            tried = null; quietFor = null; linkedIn = null
            return null
        }
        if (quietFor != project) quietFor = null
        if (situation.linked) {
            linkedIn = project
            return null
        }
        if (linkedIn != null) {
            if (linkedIn == project) quietFor = project
            linkedIn = null
        }
        val peer = situation.peer
        if (!situation.enabled || situation.busy || peer == null || quietFor == project) return null
        val attempt = Triple(project, peer, situation.network)
        if (tried == attempt) return null
        tried = attempt
        return peer
    }

    /** The connection [next] asked for now exists. Said at once, so that its
     * disappearance is noticed even if it goes before the next step looks. */
    public fun started(project: String) {
        linkedIn = project
    }

    /** The app could not act on the answer from [next] (it had become busy):
     * the same attempt may be made again. */
    public fun retry() {
        tried = null
    }

    /** The owner asked for automatic connection again. */
    public fun resume() {
        tried = null; quietFor = null
    }

    /** The project was closed. A project opened later is a new decision, even
     * when it is the same project and no step ran in between. */
    public fun closed() {
        tried = null; quietFor = null; linkedIn = null
    }

    /** True while it is staying out of the way for this project. */
    public fun isQuietFor(project: String?): Boolean = project != null && quietFor == project
}
