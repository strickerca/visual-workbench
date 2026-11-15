package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** What the connection sheet shows about automatic connection. It carries no
 * address, device identity or adapter detail beyond the owner's own choice. */
internal data class AutomaticStatus(
    val enabled: Boolean = AUTOMATIC_BY_DEFAULT,
    val tetherAdapter: String? = null,
    /** A connection that was started automatically exists. */
    val active: Boolean = false,
    val hosting: Boolean = false,
    /** Local addresses the automatic listener is bound to. */
    val listening: Int = 0,
    val note: AutomaticNote? = null,
    /** It is staying out of the way after the connection was ended by hand. */
    val quiet: Boolean = false,
    /** The adapters the owner chose for automatic listening, by system name. */
    val adapters: List<String> = emptyList(),
)

internal enum class AutomaticNote { ChoosePhone, DifferentHost, ChooseNetwork, NoNetwork, CouldNotStart }

/** The few things automatic connection needs from the editor. */
internal interface AutomaticEditor {
    /** Identity of the open project instance, or null. */
    val project: String?
    val role: ProjectSessionRole?
    /** The status of the project connection, or null when there is none. */
    val status: SessionStatus?
    val busy: Boolean
    fun inspectRole()
    /** Listen on [endpoints] when this device hosts the project, otherwise
     * dial remembered routes with [endpoints] as extra hints. */
    fun start(peer: String, endpoints: List<SessionEndpoint>, tether: () -> UInt?, finished: (Exception?) -> Unit): Boolean
    /** The live connection, if it is one that [start] created. */
    fun automaticLink(): ProjectLink?
    /** Tell the owner one thing they can do, in the editor's own status line. */
    fun hint(message: String)
}

internal class EditorAutomaticPort(private val editor: EditorController, private val sessions: DesktopSessionController) : AutomaticEditor {
    override val project: String? get() = editor.state.value.let { state ->
        state.document?.takeIf { !state.isStreamDraft }?.let { "${state.projectEpoch}:${it.render.revision.projectId}" }
    }
    override val role: ProjectSessionRole? get() = editor.state.value.sessionRole
    override val status: SessionStatus? get() = editor.state.value.sync
    override val busy: Boolean get() = editor.state.value.busy
    override fun inspectRole() = editor.inspectSessionRole(sessions)
    override fun start(peer: String, endpoints: List<SessionEndpoint>, tether: () -> UInt?, finished: (Exception?) -> Unit): Boolean =
        editor.connectAutomatically(sessions, peer, endpoints, tether, finished)
    override fun automaticLink(): ProjectLink? = editor.automaticLink()
    override fun hint(message: String) = editor.report(message)
}

/** The USB adapter is the one the owner chose in the connection sheet,
 * recognised again by the name the system gives it. */
internal fun chosenTetherAdapter(saved: SavedRoutes): (LocalSessionAddress) -> Boolean =
    { address -> address.interfaceName != null && address.interfaceName == saved.tetherAdapter }

/** The automatic listener uses only adapters the owner chose in the connection
 * sheet (for pairing, for the USB project, or explicitly), never every
 * network the computer happens to be on. */
internal fun chosenAdapters(saved: SavedRoutes): (LocalSessionAddress) -> Boolean =
    { address -> address.interfaceName != null && address.interfaceName in saved.adapters }

/** Starts and looks after the project connection so the owner does not have
 * to: when a project is open and a paired phone exists, the computer listens
 * (or, for a project the phone hosts, dials) by itself. It only ever starts a
 * connection; an explicit Disconnect or a manual connection is left alone
 * until the project is opened again or automatic connection is switched back
 * on. Trust is unchanged: every connection is the same pinned-certificate
 * session the manual buttons create. While the owner's switch is off (see
 * [AUTOMATIC_BY_DEFAULT]) it does nothing at all. Runs on the UI thread. */
internal class DesktopAutomaticConnection(
    parent: CoroutineScope,
    private val sessions: DesktopSessionController,
    private val editor: AutomaticEditor,
    private val keeper: AutomaticRouteKeeper,
    private val interval: Long = 2000,
) {
    private val lifetime = SupervisorJob(parent.coroutineContext[Job])
    private val scope = CoroutineScope(parent.coroutineContext + lifetime)
    private val mutable = MutableStateFlow(AutomaticStatus())
    val state: StateFlow<AutomaticStatus> = mutable
    private val decision = AutomaticConnection()
    private var loop: Job? = null
    private var roleAskedFor: String? = null
    private var tetherInterface: UInt? = null
    private var note: AutomaticNote? = null
    private var keeping = false
    private var hinted = false

    fun start() {
        if (loop != null) return
        loop = scope.launch {
            while (isActive) {
                try { step() }
                catch (cancel: CancellationException) { throw cancel }
                catch (_: Exception) { /* Automatic connection never interrupts the owner; the next step tries again. */ }
                delay(interval)
            }
        }
    }

    fun setEnabled(value: Boolean) {
        scope.launch {
            val saved = keeper.change { it.withAutomatic(value) }
            if (value) decision.resume()
            publish(saved)
        }
    }

    /** Called when the owner picks the adapter for the USB project connection.
     * Nothing is kept while automatic connection is switched off. */
    fun rememberTetherAdapter(address: LocalSessionAddress?): Job? {
        val name = address?.takeIf { !it.loopback }?.interfaceName ?: return null
        return scope.launch { publish(keeper.change { if (it.automatic) it.withTetherAdapter(name) else it }) }
    }

    /** Called when the owner picks a PC network connection in the connection
     * sheet: to pair on it, to connect a project on it, or just to allow it.
     * Only such adapters are listened on automatically. Nothing is kept while
     * automatic connection is switched off. */
    fun rememberAdapter(address: LocalSessionAddress?): Job? {
        val name = address?.takeIf { !it.loopback }?.interfaceName ?: return null
        return scope.launch { publish(keeper.change { if (it.automatic) it.withAdapter(name) else it }); decision.retry() }
    }

    fun forgetAdapters() {
        scope.launch { publish(keeper.change { it.withoutAdapters() }) }
    }

    private fun publish(saved: SavedRoutes) {
        mutable.value = mutable.value.copy(enabled = saved.automatic, tetherAdapter = saved.tetherAdapter, adapters = saved.adapters.toList())
    }

    suspend fun step() {
        val session = sessions.state.value
        val setting = keeper.saved()
        if (!session.ready || !setting.automatic) {
            // Switched off: nothing is listened on, announced, looked for,
            // dialled or saved. A connection that already exists is left alone.
            if (keeping) { keeping = false; keeper.release() }
            decision.closed(); note = null; roleAskedFor = null
            mutable.value = AutomaticStatus(setting.automatic, setting.tetherAdapter, adapters = setting.adapters.toList())
            return
        }
        val service = sessions.requireService()
        keeper.restore(service, session.paired)
        val plan = keeper.plan()
        tetherInterface = plan.tetherInterface
        val project = editor.project
        val role = editor.role
        if (project == null) roleAskedFor = null
        else if (role == null && !editor.busy && roleAskedFor != project) { roleAskedFor = project; editor.inspectRole() }
        val active = session.paired.filter { it.revokedAtMs == null }
        val peer = if (project != null && role != null) automaticPeer(session.paired, role) { plan.saved.isLastPeer(keeper.key(it)) } else null
        val status = editor.status
        val target = decision.next(AutomaticSituation(plan.saved.automatic, project?.takeIf { role != null }, peer?.deviceId,
            status != null, editor.busy || session.pairing, plan.network))
        if (project == null || role == null || status != null || !plan.saved.automatic) note = null
        else if (peer == null) note = when {
            !role.isHost -> AutomaticNote.DifferentHost
            active.size > 1 -> AutomaticNote.ChoosePhone
            else -> null
        }
        if (target != null && peer != null && role != null && project != null) {
            // Listening happens only on adapters the owner chose; with none
            // chosen yet, or none of them present, there is nothing to start.
            if (role.isHost && plan.binds.isEmpty()) {
                note = if (plan.saved.adapters.isEmpty() && plan.saved.tetherAdapter == null) AutomaticNote.ChooseNetwork else AutomaticNote.NoNetwork
                // Said once per run, where the owner looks: the one choice that is still theirs.
                if (note == AutomaticNote.ChooseNetwork && !hinted) {
                    hinted = true
                    editor.hint("To let your paired phone connect by itself, open Pair devices and choose this PC's network connection once.")
                }
            } else {
                note = null
                val accepted = editor.start(peer.deviceId, if (role.isHost) plan.binds else emptyList(), { tetherInterface }) { error ->
                    if (error != null) note = AutomaticNote.CouldNotStart else decision.started(project)
                }
                // The editor was busy after all: ask again at the next step.
                if (!accepted) decision.retry()
            }
        }
        val link = editor.automaticLink()
        val linkPeer = status?.let { current -> active.firstOrNull { it.deviceId == current.peerDeviceId } }
        if (link != null && linkPeer != null && role != null) {
            keeping = true
            keeper.keep(service, link, role.isHost, linkPeer, status, plan)
        } else {
            if (keeping) { keeping = false; keeper.release() }
            // A connection made with the buttons counts too: where it
            // authenticated is tried first next time, and the phone it reached
            // is the one to prefer when several are paired.
            if (linkPeer != null) keeper.authenticated(service, linkPeer, status)
        }
        val saved = keeper.saved()
        mutable.value = AutomaticStatus(saved.automatic, saved.tetherAdapter, link != null, role?.isHost == true,
            if (link != null && role?.isHost == true) try { link.endpoints.size } catch (_: SessionFailure) { 0 } else 0, note,
            decision.isQuietFor(project), saved.adapters.toList())
    }

    suspend fun close() = withContext(NonCancellable) {
        loop?.cancelAndJoin(); loop = null
        try { keeper.release() } catch (_: Exception) { }
        lifetime.cancel()
    }
}

/** One plain sentence for the connection sheet. */
internal fun automaticSummary(automatic: AutomaticStatus, projectOpen: Boolean, pairedPhones: Int, sync: SessionStatus?): String = when {
    !automatic.enabled -> "Automatic connection is off. Switch it on and your paired phone connects by itself. Until then, use the buttons below."
    pairedPhones == 0 -> "Pair a phone once. After that it connects by itself."
    !projectOpen -> "Open a project and your paired phone connects to it by itself."
    automatic.active && sync != null && sync.carrier != null && sync.status in setOf(SyncStatus.Syncing, SyncStatus.Synced) ->
        "Connected to your phone automatically over ${if (sync.carrier == SessionCarrier.QuicTether) "USB tethering" else if (sync.carrier == SessionCarrier.QuicWifi) "Wi-Fi" else "the developer link"}."
    automatic.active && automatic.hosting ->
        "Waiting for your phone. It connects by itself when its app is open on the same Wi-Fi or over USB tethering."
    automatic.active -> ErrorCatalog.link(sync?.failure)?.let { "${it.whatHappened} ${it.whatToDo}" }
        ?: "Looking for the device that hosts this project. It connects by itself once found."
    sync != null -> "The project connection below was started by hand. Automatic connection resumes when the project is opened again."
    automatic.note == AutomaticNote.ChoosePhone -> "More than one phone is paired. Choose one below and connect once; it is remembered."
    automatic.note == AutomaticNote.DifferentHost -> "This project belongs to a device that is not paired with this computer."
    automatic.note == AutomaticNote.ChooseNetwork -> "Choose the PC network connection your phone is on once, below, and allow it. After that the phone connects by itself."
    automatic.note == AutomaticNote.NoNetwork -> "The chosen PC network connection is not available right now. Turn on Wi-Fi or USB tethering and the phone connects by itself."
    automatic.note == AutomaticNote.CouldNotStart -> "The automatic connection could not start. Use the buttons below; your project is unchanged."
    automatic.quiet -> "The project connection was closed. Switch automatic connection off and on, or reopen the project, to start it again."
    else -> "Getting ready to connect to your phone."
}
