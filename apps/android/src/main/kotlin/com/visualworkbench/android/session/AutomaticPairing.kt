package com.visualworkbench.android.session

import com.visualworkbench.shared.*
import kotlinx.coroutines.delay

/** One plain sentence about automatic connection for the Connect screen. */
internal fun automaticConnectionLine(automatic: Boolean, quiet: Boolean, manual: Boolean, paired: Boolean, status: SessionStatus?): String = when {
    !automatic -> "Automatic connection is off. Switch it on and this phone finds your paired computer by itself. Until then, use Connection details below."
    manual -> "A typed address is in use, so this connection is manual. Clear it in Connection details to connect automatically."
    !paired -> "Pair a computer once. After that this phone finds it by itself."
    status != null && status.carrier != null && (status.status == SyncStatus.Syncing || status.status == SyncStatus.Synced) ->
        "Connected to your computer over ${when (status.carrier) { SessionCarrier.QuicTether -> "USB tethering"; SessionCarrier.QuicWifi -> "Wi-Fi"; else -> "the developer link" }}."
    status != null -> ErrorCatalog.link(status.failure)?.let { "${it.whatHappened} ${it.whatToDo}" }
        ?: "Looking for your computer. It connects by itself over Wi-Fi or USB tethering."
    quiet -> "Disconnected. Tap Connect current project, or open the project again, to connect."
    else -> "No address is needed. Open a project on the computer, then tap Receive computer project. An open project connects by itself."
}

/** What the app shows when it asks whether to connect by itself. Read at the
 * moment of the decision, never before a suspension. */
internal class AutomaticView(
    /** Identity of the open project, or null when none is open. */
    val project: String?,
    /** The status of the project connection, or null when there is none. */
    val status: SessionStatus?,
    val busy: Boolean,
    /** The owner typed a session address: the manual flow is in charge. */
    val manual: Boolean,
)

/** Pair once, then no typing. Decides when the phone starts its project
 * connection by itself, and lets the editor's ordinary connect, host and
 * receive calls work without a typed address: the phone listens on its own
 * links or dials where the paired computer was last authenticated and where
 * discovery finds it. Trust is unchanged; every route is the same
 * pinned-certificate session. Not thread-safe: call from the main thread. */
internal class AutomaticPairing(
    private val keeper: AutomaticRouteKeeper,
    private val discovery: RouteDiscovery,
    private val pause: suspend (Long) -> Unit = { delay(it) },
) {
    // Never a real address: if it ever reached the core it would be refused.
    private val marker = listOf(SessionEndpoint(SessionCarrier.QuicWifi, "automatic"))
    private val decision = AutomaticConnection()
    private var link: ProjectLink? = null
    private var hosting = false
    private var linkPeer: String? = null
    /** The project the newest decision to connect was made for. */
    private var asked: String? = null
    /** Announcing or looking may be running for the automatic connection. */
    private var upkeep = false
    var enabled = AUTOMATIC_BY_DEFAULT; private set
    /** True while it stays out of the way after the owner ended a connection. */
    var quiet = false; private set

    /** What the editor is given as "endpoints" when no address was typed. */
    val endpoints: List<SessionEndpoint> get() = marker
    fun isAutomatic(endpoints: List<SessionEndpoint>): Boolean =
        endpoints === marker || (endpoints.size == 1 && endpoints[0].address == marker[0].address)

    /** The service the editor uses. Calls made with [endpoints] take automatic
     * routes; every other call goes to [real] unchanged. */
    fun routed(real: WorkbenchSessions): WorkbenchSessions = Routed(real)

    suspend fun setEnabled(value: Boolean) {
        enabled = keeper.change { it.withAutomatic(value) }.automatic
        if (value) { decision.resume(); quiet = false } else idle()
    }

    /** Read the owner's saved setting. While it is off nothing is announced,
     * looked for, dialled or saved, and [step] must not be called. */
    suspend fun load(): Boolean {
        enabled = keeper.saved().automatic
        if (!enabled) idle()
        return enabled
    }

    private suspend fun idle() {
        if (upkeep) { upkeep = false; keeper.release() }
        decision.closed(); quiet = false; asked = null
    }

    fun projectClosed() { decision.closed(); quiet = false; asked = null }

    /** The editor could not act on the newest answer from [step]. */
    fun missed() { decision.retry(); asked = null }

    /** The pairing address the phone just used is the first place to look.
     * Nothing is kept while automatic connection is switched off. */
    suspend fun learned(service: WorkbenchSessions, device: PairedDevice, address: String) {
        if (load()) keeper.learned(service, device, address)
    }

    /** Stop announcing and looking (the app left the foreground). A live
     * connection is not touched. */
    suspend fun rest() { upkeep = false; keeper.release() }

    /** One step: look after the automatic connection if there is one, and
     * return the paired computer to connect to now, or null. [service] is the
     * real service, not the routed one. */
    suspend fun step(service: WorkbenchSessions, paired: () -> List<PairedDevice>, view: () -> AutomaticView): String? {
        if (!load()) return null
        keeper.restore(service, paired())
        val plan = keeper.plan()
        // No suspension from here to the decision: the view is current.
        val now = view()
        val devices = paired()
        val peer = automaticPeer(devices, null) { plan.saved.isLastPeer(keeper.key(it)) }
        // With no network at all neither listening nor dialling can work; wait
        // for one instead of showing the owner a failure they did not ask for.
        val target = decision.next(AutomaticSituation(enabled && !now.manual && plan.binds.isNotEmpty(), now.project,
            peer?.deviceId, now.status != null, now.busy, plan.network))
        quiet = decision.isQuietFor(now.project)
        if (target != null) asked = now.project
        val current = link
        if (now.status == null) {
            if (current != null) { link = null; upkeep = false; keeper.release() }
        } else if (current != null) {
            devices.firstOrNull { it.deviceId == linkPeer && it.revokedAtMs == null }
                ?.let { upkeep = true; keeper.keep(service, current, hosting, it, now.status, plan) }
        } else {
            // A connection made with a typed address counts too: where it
            // authenticated is tried first next time.
            devices.firstOrNull { it.deviceId == now.status.peerDeviceId && it.revokedAtMs == null }
                ?.let { keeper.authenticated(service, it, now.status) }
        }
        return target
    }

    private fun adopt(created: ProjectLink, peer: String, host: Boolean) {
        link = created; linkPeer = peer; hosting = host
        // Said at once, so an early disconnect is still seen as the owner's.
        asked?.let(decision::started)
        asked = null
    }

    /** Discovery may need a moment after the app opens; give it one before a
     * first receive, when nothing is remembered yet. */
    private suspend fun hintsFor(real: WorkbenchSessions, peer: String): List<SessionEndpoint> {
        repeat(16) {
            val plan = keeper.plan()
            upkeep = true
            val hints = classifyRouteHints(discovery.found(real, plan.usable), plan.local, plan.tether)
            if (hints.isNotEmpty() || real.knownRoutes(peer).isNotEmpty()) return hints
            pause(250)
        }
        return emptyList()
    }

    private inner class Routed(private val real: WorkbenchSessions) : WorkbenchSessions by real {
        override suspend fun host(project: WorkbenchProject, peer: String, endpoints: List<SessionEndpoint>): ProjectLink {
            if (!isAutomatic(endpoints)) { link = null; asked = null; return real.host(project, peer, endpoints) }
            val plan = keeper.plan()
            // Nothing to listen on: refuse rather than pretend to be reachable.
            if (plan.binds.isEmpty()) throw SessionFailure(SessionFailureKind.Transport)
            return real.hostAutomatic(project, peer, plan.binds).also { adopt(it, peer, true) }
        }
        override suspend fun connect(project: WorkbenchProject, peer: String, endpoints: List<SessionEndpoint>): ProjectLink {
            if (!isAutomatic(endpoints)) { link = null; asked = null; return real.connect(project, peer, endpoints) }
            return real.connectAutomatic(project, peer, emptyList()).also { adopt(it, peer, false) }
        }
        override suspend fun receiveProject(path: String, peer: String, endpoints: List<SessionEndpoint>, expectedProjectId: String?, maxBlobBytes: ULong): WorkbenchProject =
            if (!isAutomatic(endpoints)) real.receiveProject(path, peer, endpoints, expectedProjectId, maxBlobBytes)
            else real.receiveProjectAutomatic(path, peer, hintsFor(real, peer), expectedProjectId, maxBlobBytes)
    }
}
