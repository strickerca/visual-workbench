package com.visualworkbench.shared

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The platform's way to announce a listener and to look for one nearby.
 * Everything it reports is an untrusted address hint. */
public interface RouteDiscovery {
    /** Announce these listening addresses, replacing any earlier
     * announcement. An empty list stops announcing. Must not throw when
     * discovery is unavailable: the connection then relies on remembered routes. */
    public suspend fun advertise(service: WorkbenchSessions, endpoints: List<String>)
    /** Addresses announced nearby right now, looked for on the links in [local]. */
    public suspend fun found(service: WorkbenchSessions, local: List<LocalSessionAddress>): List<String>
    public suspend fun stop()
}

/** Discovery that the native core runs by itself (the Windows computer). */
public class NativeRouteDiscovery : RouteDiscovery {
    private var announcing: SessionDiscovery? = null
    private var announced: List<String>? = null
    private var browsing: SessionDiscovery? = null
    private var browsingOn: List<String>? = null

    override suspend fun advertise(service: WorkbenchSessions, endpoints: List<String>) {
        if (endpoints == announced) return
        closeQuietly(announcing); announcing = null
        announced = endpoints
        if (endpoints.isNotEmpty()) announcing = try { service.createDiscovery(endpoints) } catch (_: SessionFailure) { null }
    }

    override suspend fun found(service: WorkbenchSessions, local: List<LocalSessionAddress>): List<String> {
        val on = local.filter { !it.loopback && ':' !in it.address }.map { it.address }.distinct().take(8)
        if (on != browsingOn) {
            closeQuietly(browsing); browsing = null
            browsingOn = on
            if (on.isNotEmpty()) browsing = try { service.browseDiscovery(on) } catch (_: SessionFailure) { null }
        }
        val current = browsing ?: return emptyList()
        return try { current.snapshot().flatMap { it.endpoints } } catch (_: SessionFailure) { emptyList() }
    }

    override suspend fun stop() {
        closeQuietly(announcing); closeQuietly(browsing)
        announcing = null; browsing = null; announced = null; browsingOn = null
    }

    private suspend fun closeQuietly(discovery: SessionDiscovery?) {
        try { discovery?.close() } catch (cancel: CancellationException) { throw cancel } catch (_: Exception) { }
    }
}

/** What an automatic connection may use right now. */
public class AutomaticPlan internal constructor(
    public val saved: SavedRoutes,
    public val local: List<LocalSessionAddress>,
    /** True for an address on the one local USB tethering link, if there is one. */
    public val tether: (LocalSessionAddress) -> Boolean,
    /** The local addresses on links this app may listen and look on: Wi-Fi
     * and USB tethering, never every interface the device has. */
    public val usable: List<LocalSessionAddress>,
) {
    /** The local addresses to listen on, USB tethering first. */
    public val binds: List<SessionEndpoint> = automaticBindEndpoints(usable, tether)
    /** Changes whenever the addresses to listen on change. */
    public val network: String get() = binds.joinToString("|") { "${it.carrier.ordinal} ${it.address}" }
    public val tetherInterface: UInt? get() = local.filter(tether).map { it.interfaceIndex }.distinct().singleOrNull()
    override fun toString(): String = "AutomaticPlan([REDACTED])"
}

/** Keeps an automatically started connection supplied with what it needs and
 * saves what it learns: restores saved routes into the core, keeps a host's
 * listeners matched to the local network and announced, feeds a client the
 * addresses discovery finds, and stores the route once it has authenticated.
 * Nothing here trusts an address: the core dials every hint with the paired
 * device's pinned certificate. Call from one thread (the UI thread). */
public class AutomaticRouteKeeper(
    private val store: KnownRouteStore,
    private val discovery: RouteDiscovery,
    /** Says which local addresses are on a USB tethering link. On the phone
     * that is its own tethering interface; on the computer, the adapter the
     * owner chose for USB. */
    private val tetherLink: (SavedRoutes) -> (LocalSessionAddress) -> Boolean,
    /** Says which local addresses the app may listen and look on. The sync
     * listener belongs on Wi-Fi and USB tethering links only: on the phone
     * those are its Wi-Fi and tethering interfaces; on the computer, the
     * adapters the owner chose. */
    private val usableLink: (SavedRoutes) -> (LocalSessionAddress) -> Boolean,
    private val localAddresses: suspend () -> List<LocalSessionAddress> = ::localSessionAddresses,
) {
    private var restoredInto: WorkbenchSessions? = null
    private val restored = HashSet<String>()
    private var kept: ProjectLink? = null
    private var listening: List<SessionEndpoint>? = null
    private var offered: List<SessionEndpoint>? = null
    private var offeredOn: String? = null
    private var remembered: Pair<String, List<SessionEndpoint>>? = null
    /** Addresses this run was given at pairing, by device id, that the core
     * has not authenticated yet. Memory only: offered as hints, never saved
     * and never put among the core's remembered routes. */
    private val pairingHints = HashMap<String, String>()

    public suspend fun saved(): SavedRoutes = withContext(Dispatchers.IO) { store.load() }
    public suspend fun change(change: (SavedRoutes) -> SavedRoutes): SavedRoutes = withContext(Dispatchers.IO) { store.update(change) }
    public fun key(device: PairedDevice): String? = KnownRouteStore.peerKey(device.deviceId, device.fingerprint)

    /** Drop everything kept for a device that is not paired any more (revoked,
     * or simply unknown): its saved routes, the note that it was used last,
     * and a pairing hint not yet used. Call it whenever the list of paired
     * devices is read or changes, whether automatic connection is on or off. */
    public suspend fun prune(paired: List<PairedDevice>) {
        val active = paired.filter { it.revokedAtMs == null }
        val keys = active.mapNotNull(::key).toSet()
        val saved = saved()
        if (saved.peers.any { it !in keys } || saved.lastPeer?.let { it !in keys } == true) change { it.keepOnly(keys) }
        restored.retainAll(keys)
        pairingHints.keys.retainAll(active.map { it.deviceId }.toSet())
    }

    /** Hand the core the routes saved by earlier runs, once per paired device,
     * and drop whatever belongs to devices that are no longer paired. */
    public suspend fun restore(service: WorkbenchSessions, paired: List<PairedDevice>) {
        if (restoredInto !== service) { restoredInto = service; restored.clear() }
        prune(paired)
        val saved = saved()
        for (device in paired.filter { it.revokedAtMs == null }) {
            val key = key(device) ?: continue
            if (!restored.add(key)) continue
            val routes = saved.routesFor(key)
            if (routes.isEmpty()) continue
            try { service.restoreRoutes(device.deviceId, routes) }
            catch (cancel: CancellationException) { throw cancel }
            // The core refused them (not numeric, not local, not paired): forget them.
            catch (_: SessionFailure) { change { it.forget(key) } }
        }
    }

    public suspend fun plan(): AutomaticPlan {
        val saved = saved()
        val local = try { localAddresses() } catch (cancel: CancellationException) { throw cancel } catch (_: Exception) { emptyList() }
        val candidate = tetherLink(saved)
        // Two links that both look like USB tethering: label neither.
        val single = local.filter(candidate).map { it.interfaceIndex }.distinct().size == 1
        val tether: (LocalSessionAddress) -> Boolean = if (single) candidate else { _ -> false }
        val usable = usableLink(saved)
        return AutomaticPlan(saved, local, tether, local.filter { !it.loopback && (usable(it) || tether(it)) })
    }

    /** An address this device was just given for a paired device: the one in
     * the pairing code it scanned. Pairing proved the device at the pairing
     * listener, not at its project listener, so the address is only a place to
     * try. It is offered to the core as a hint for the rest of this run and is
     * saved only once the core has authenticated the device there. */
    public fun offerFromPairing(device: PairedDevice, address: String) {
        val text = address.trim()
        if (device.revokedAtMs == null && key(device) != null && isNumericSessionEndpoint(text)) pairingHints[device.deviceId] = text
    }

    /** What to offer the core for [peer] besides the routes it remembers: an
     * address given at pairing and, with [look], what discovery finds now.
     * Every one is an untrusted hint. */
    public suspend fun hints(service: WorkbenchSessions, peer: String, plan: AutomaticPlan, look: Boolean = true): List<SessionEndpoint> =
        classifyRouteHints(listOfNotNull(pairingHints[peer]) + (if (look) discovery.found(service, plan.usable) else emptyList()),
            plan.local, plan.tether)

    /** One upkeep step for [link], a connection that was started automatically
     * with [peer]. [status] is its newest status. */
    public suspend fun keep(service: WorkbenchSessions, link: ProjectLink, hosting: Boolean, peer: PairedDevice,
        status: SessionStatus?, plan: AutomaticPlan) {
        if (kept !== link) { if (kept != null) release(); kept = link }
        try {
            if (hosting) {
                // With no usable address the present listeners stay as they are.
                if (plan.binds.isNotEmpty() && plan.binds != listening) {
                    val bound = link.listenOn(plan.binds)
                    // An address that could not be bound yet is tried again next time.
                    listening = plan.binds.takeIf { bound.size >= it.size }
                    discovery.advertise(service, bound.filter { it.carrier != SessionCarrier.TcpAdb }.map { it.address })
                }
            } else {
                val hints = hints(service, peer.deviceId, plan)
                // A local link that came or went (USB tethering switched on) is
                // offered again even with the same hints: that asks the core
                // for a fresh look at once instead of at its next slow retry.
                if (hints != offered || plan.network != offeredOn) { link.offerRoutes(hints); offered = hints; offeredOn = plan.network }
            }
            authenticated(service, peer, status)
        } catch (cancel: CancellationException) { throw cancel }
        catch (_: SessionFailure) { /* The link is closing or busy; the next step tries again. */ }
    }

    /** Save where the core authenticated [peer], once [status] shows a live
     * connection with it. It serves a connection made by hand as well, so an
     * address typed once is tried first from then on. */
    public suspend fun authenticated(service: WorkbenchSessions, peer: PairedDevice, status: SessionStatus?) {
        if (status?.carrier == null || (status.status != SyncStatus.Syncing && status.status != SyncStatus.Synced)) return
        val key = key(peer) ?: return
        try {
            // Only an authenticated route reaches this list.
            val routes = service.knownRoutes(peer.deviceId)
            if (remembered != key to routes) {
                change { if (routes.isEmpty()) it.withLastPeer(key) else it.remember(key, routes) }
                remembered = key to routes
            }
            // The address from pairing has now authenticated: it is a route, no longer a hint.
            if (routes.any { it.address == pairingHints[peer.deviceId] }) pairingHints.remove(peer.deviceId)
        } catch (cancel: CancellationException) { throw cancel }
        catch (_: SessionFailure) { /* Asked again at the next step. */ }
    }

    /** The automatic connection is gone: stop announcing and looking. */
    public suspend fun release() {
        discovery.stop()
        kept = null; listening = null; offered = null; offeredOn = null; remembered = null
    }
}
