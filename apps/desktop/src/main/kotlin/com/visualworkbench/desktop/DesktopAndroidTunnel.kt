package com.visualworkbench.desktop

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogWindow
import com.visualworkbench.bindings.host.excludeCaptureWindows
import com.visualworkbench.desktop.capture.deliverCaptureProject
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path

internal data class DesktopTunnelState(
    val open: Boolean = false, val busy: Boolean = false, val connected: Boolean = false,
    val mode: TunnelMode? = null, val target: TunnelTarget? = null,
    val cleanupPending: Boolean = false, val stopping: Boolean = false, val message: String? = null,
)

/** Desktop-private seams for bounded ownership tests. Production always uses
 * native compiled admission; no path, report or caller option admits a runtime. */
internal class DesktopTunnelFactories(
    val available: () -> Boolean = ::androidTunnelAvailable,
    val open: suspend (TunnelOptions) -> WorkbenchAndroidTunnel = ::openAndroidTunnel,
    val retryRetirement: suspend () -> Unit = ::retryAndroidTunnelRetirement,
    val assertRetired: () -> Unit = ::assertAndroidTunnelRetired,
    val semantics: (WorkbenchProject) -> WorkbenchSemantics = ::semanticWorkflows,
    val deleteWork: (Path) -> Unit = { Files.delete(it) },
)

/** Local owner only. Failed cleanup stays reachable and seals action admission
 * until close joins every admitted producer and proves actual retirement. */
internal class DesktopAndroidTunnel(
    private val scope: CoroutineScope, private val core: WorkbenchCore,
    private val device: String, private val projects: Path, private val runtime: Path,
    private val beforeOpen: suspend () -> Unit,
    private val route: () -> (suspend (WorkbenchProject, Path, String?) -> Unit),
    private val modal: (Boolean) -> Unit, private val mayOpen: () -> Boolean, private val report: (String) -> Unit,
    private val factories: DesktopTunnelFactories = DesktopTunnelFactories(),
) {
    private val mutable = MutableStateFlow(DesktopTunnelState())
    val state = mutable.asStateFlow()
    private val admission = Any()
    private val retirement = Mutex()
    private var stopping = false
    private var closers = 0
    private var owner: WorkbenchAndroidTunnel? = null
    private var operation: Job? = null
    private var nativeAttempted = false
    private var heldProject: WorkbenchProject? = null
    private var heldCapture: WorkbenchTunnelCapture? = null
    private var heldCollection: WorkbenchSemanticCollection? = null
    private var heldPlan: WorkbenchWorkflowPlan? = null
    private var heldWork: Path? = null

    fun show() = synchronized(admission) {
        if (stopping || !scope.isActive || mutable.value.open || mutable.value.cleanupPending || !mayOpen()) return@synchronized
        val message = if (factories.available())
            "Choose the owner's platform-tools adb and exact USB model and transport."
        else "Android mirroring is unavailable in this package."
        modal(true)
        mutable.update { it.copy(open = true, message = message) }
    }

    private fun launch(
        admit: () -> Boolean = { true }, onAdmitted: () -> Unit = {}, body: suspend () -> Unit,
    ) {
        val next = synchronized(admission) {
            if (stopping || !scope.isActive || !mutable.value.open || operation?.isCompleted == false ||
                mutable.value.cleanupPending || !admit()) return
            onAdmitted()
            mutable.update { it.copy(busy = true, message = null) }
            scope.launch(start = CoroutineStart.LAZY) {
                try { body() }
                catch (error: CancellationException) {
                    markFailure("Android work was cancelled. Close the mirror before retrying.")
                    throw error
                } catch (_: Exception) {
                    markFailure("The Android operation was refused. Review the selected device or close and retry.")
                }
            }.also { job ->
                operation = job
                job.invokeOnCompletion {
                    synchronized(admission) {
                        if (operation === job) {
                            operation = null
                            mutable.update { it.copy(busy = stopping) }
                        }
                    }
                }
            }
        }
        next.start()
    }

    private fun markFailure(message: String) = mutable.update {
        it.copy(cleanupPending = it.cleanupPending || nativeAttempted || heldProject != null ||
            heldCapture != null || heldCollection != null || heldPlan != null || heldWork != null,
            target = null, message = message)
    }

    private suspend fun publish(change: (DesktopTunnelState) -> DesktopTunnelState) {
        currentCoroutineContext().ensureActive()
        synchronized(admission) { if (!stopping) mutable.update(change) }
    }

    /** Retain in the producer, before an awaited result can be cancelled at its
     * handoff. coroutineScope joins that child before caller cleanup proceeds. */
    private suspend fun <T> acquireOwned(retain: (T) -> Unit, create: suspend () -> T): T = coroutineScope {
        currentCoroutineContext().ensureActive()
        async(start = CoroutineStart.UNDISPATCHED) { create().also(retain) }.await()
    }

    fun connect(adb: String, model: String, transport: UInt) = launch(admit = { owner == null }) {
        if (!factories.available()) {
            publish { it.copy(message = "Android mirroring is unavailable in this package.") }
            return@launch
        }
        beforeOpen()
        currentCoroutineContext().ensureActive()
        nativeAttempted = true // An ownerless failed acquisition can still need native retirement.
        val opened = acquireOwned({ synchronized(admission) { owner = it } }) {
            factories.open(TunnelOptions(true, runtime.toString(), adb, model, transport,
                core.newId(System.currentTimeMillis().toULong())))
        }
        currentCoroutineContext().ensureActive()
        val actual = opened.state()
        require(actual.mode == TunnelMode.ViewOnly)
        publish { it.copy(connected = true, mode = actual.mode,
            message = "View only. Phone input requires a fresh local confirmation.") }
    }

    fun requestControl() = launch(admit = {
        mutable.value.connected && mutable.value.mode == TunnelMode.ViewOnly && mutable.value.target == null
    }) {
        beforeOpen()
        currentCoroutineContext().ensureActive()
        val target = checkNotNull(owner).beginControlConfirmation()
        publish { it.copy(target = target) }
    }

    /** The event carries the exact displayed object. A stale queued UI event
     * cannot consume a replacement confirmation, even with equal field values. */
    fun confirmControl(target: TunnelTarget) = launch(
        admit = { mutable.value.target === target },
        onAdmitted = { mutable.update { it.copy(target = null) } },
    ) {
        beforeOpen()
        currentCoroutineContext().ensureActive()
        val actual = checkNotNull(owner).confirmControl(target)
        require(actual.mode == TunnelMode.OwnerControl)
        publish { it.copy(mode = actual.mode, message = "Owner control enabled for the confirmed USB device.") }
    }

    fun cancelConfirmation(target: TunnelTarget) = launch(
        admit = { mutable.value.target === target },
        onAdmitted = { mutable.update { it.copy(target = null) } },
    ) { checkNotNull(owner).cancelControlConfirmation() }

    fun revokeControl() = launch(admit = { mutable.value.connected && mutable.value.target == null }) {
        beforeOpen()
        currentCoroutineContext().ensureActive()
        val actual = checkNotNull(owner).revokeControl()
        if (actual.mode == TunnelMode.Closed) {
            withContext(NonCancellable) { retireSession() }
            publish { it.copy(connected = false, mode = actual.mode, target = null,
                message = "Phone input revoked and mirror closed. Open a fresh view-only connection to continue.") }
        } else {
            require(actual.mode == TunnelMode.ViewOnly)
            publish { it.copy(mode = actual.mode, target = null, message = "Phone input revoked.") }
        }
    }

    fun capture() {
        // Pin the destination during admission, before any native acquisition.
        // This factory returns a borrow-free route bound to the current editor.
        var destinationRoute: (suspend (WorkbenchProject, Path, String?) -> Unit)? = null
        launch(admit = { mutable.value.connected && mutable.value.target == null },
            onAdmitted = { destinationRoute = route() }) {
            val deliver = checkNotNull(destinationRoute)
            var deliveryFailed = false
            try {
                val captured = acquireOwned({ heldCapture = it }) { checkNotNull(owner).capture() }
                val info = captured.info()
                val now = System.currentTimeMillis()
                fun id() = core.newId(now.toULong())
                val projectId = id()
                val document = id()
                val destination = projects.resolve("$projectId.vwb")
                val work = withContext(Dispatchers.IO) {
                    createWorkDirectory(projects).also { heldWork = it }
                }
                val opened = acquireOwned({ heldProject = it }) {
                    // Leave room in the same process pool for the retained PNG,
                    // hierarchy and actor response during the actual import job.
                    captured.createProject(CreateFileProject(destination.toString(), projectId, document, id(),
                        device, "Android capture", "", work.toString(), now, memoryBudgetBytes = 128uL * 1024uL * 1024uL))
                }
                var warning = when (info.hierarchy) {
                    TunnelHierarchy.Missing -> "The screenshot has no accessibility hierarchy."
                    TunnelHierarchy.Stale -> "Accessibility was stale; only the screenshot was saved."
                    TunnelHierarchy.Present -> null
                }
                if (info.hierarchy == TunnelHierarchy.Present) {
                    try { attachHierarchy(opened, document, captured, info) }
                    catch (error: CancellationException) { throw error }
                    catch (_: Exception) { warning = "The screenshot was saved; accessibility information could not be attached." }
                    // A failed semantic child close must retain its project parent.
                    check(heldPlan == null && heldCollection == null)
                }
                captured.close()
                heldCapture = null
                releaseWork()
                currentCoroutineContext().ensureActive()
                // The canonical destination owns project close at invocation.
                // An escaping ordinary error means its close was unproved.
                heldProject = null
                deliverCaptureProject(opened, { deliver(it, destination, warning) }, retainFailure = { held, _ ->
                    heldProject = held
                    deliveryFailed = true
                    mutable.update { it.copy(cleanupPending = true) }
                })
                publish { it.copy(message = "Lossless Android capture saved.${warning?.let { text -> " $text" }.orEmpty()}") }
            } finally {
                withContext(NonCancellable) {
                    try { retireCapture(retryProject = !deliveryFailed) }
                    catch (error: Exception) {
                        mutable.update { it.copy(cleanupPending = true) }
                        report("Android capture cleanup is pending. Native owners remain held.")
                        throw error
                    }
                }
            }
        }
    }

    private suspend fun retireCapture(retryProject: Boolean = true) {
        // Never close a parent underneath a child whose retirement was refused.
        heldPlan?.close(); heldPlan = null
        heldCollection?.close(); heldCollection = null
        if (heldProject != null && !retryProject) return
        heldProject?.close(); heldProject = null
        heldCapture?.close(); heldCapture = null
        releaseWork()
    }

    private suspend fun releaseWork() {
        val path = heldWork ?: return
        withContext(Dispatchers.IO) {
            require(path.isAbsolute && path == path.normalize() && path.toRealPath() == path)
            factories.deleteWork(path) // Empty only; never delete an unexpected private file.
            heldWork = null // Before a cancelled dispatcher return can discard the successful deletion.
        }
    }

    private suspend fun attachHierarchy(project: WorkbenchProject, document: String,
        capture: WorkbenchTunnelCapture, info: TunnelCaptureInfo) {
        val observation = desktopTunnelHierarchy(info, capture.elements())
        val revision = project.info()
        val ticket = acquireOwned({ heldCollection = it }) {
            factories.semantics(project).beginCapture(WorkflowBinding(revision.projectId, document, revision.hostSeq, revision.stateHash))
        }
        try {
            val described = ticket.describe()
            require(described.sourceAssetId == info.assetId && described.captureSessionId == info.captureSessionId &&
                described.frameId == info.frameId && described.geometryRevision.toULong() == info.sourceGeneration &&
                described.width == info.width.toDouble() && described.height == info.height.toDouble())
            val now = System.currentTimeMillis()
            val plan = acquireOwned({ heldPlan = it }) {
                ticket.prepare(WorkflowMetadata(core.newId(now.toULong()), device, revision.nextLamport, now),
                    core.newId(now.toULong()), SemanticPlatform.AndroidAx, observation.frameDeltaMs,
                    observation.elapsedMs, SemanticBoundsSpace.CapturePixels, observation.elements)
            }
            try { plan.commit() } finally { withContext(NonCancellable) { plan.close(); heldPlan = null } }
        } finally {
            if (heldPlan == null) { ticket.close(); heldCollection = null }
        }
    }

    private suspend fun retireSession() {
        owner?.close()
        synchronized(admission) { owner = null }
        if (nativeAttempted) {
            factories.retryRetirement()
            factories.assertRetired()
            nativeAttempted = false
        }
    }

    suspend fun close(): Unit = withContext(NonCancellable) {
        // Seal before waiting for another closer. An admitted operation remains
        // reachable until joined, including an acquisition that returns late.
        synchronized(admission) {
            closers++
            stopping = true
            mutable.update { it.copy(stopping = true, busy = true, target = null) }
        }
        retirement.withLock {
            synchronized(admission) {
                stopping = true
                mutable.update { it.copy(stopping = true, busy = true, target = null) }
            }
            try {
                val running = synchronized(admission) { operation }
                val prior = synchronized(admission) { owner }
                try { prior?.cancel() } catch (_: Exception) { /* close retries a sealed native owner. */ }
                running?.cancelAndJoin()
                // Open may have produced an owner after the first cancel signal.
                val current = synchronized(admission) { owner }
                if (current !== prior) try { current?.cancel() } catch (_: Exception) { }
                retireCapture()
                retireSession()
                modal(false)
                synchronized(admission) {
                    operation = null
                    mutable.value = DesktopTunnelState()
                }
            } catch (error: Exception) {
                mutable.update { it.copy(busy = false, stopping = false, cleanupPending = true, target = null,
                    message = "Android cleanup is pending. Retry close before leaving the project.") }
                throw error
            } finally {
                synchronized(admission) {
                    closers--
                    stopping = closers > 0 || mutable.value.cleanupPending
                    mutable.update { it.copy(stopping = closers > 0, busy = closers > 0) }
                }
            }
        }
    }
}

internal data class DesktopTunnelHierarchy(val frameDeltaMs: Int, val elapsedMs: ULong,
    val elements: List<CapturedSemanticElement>)

/** Bound observations before subtraction/conversion. These are measured native
 * acquisition times and capture pixels, never physical input latency evidence. */
internal fun desktopTunnelHierarchy(info: TunnelCaptureInfo, rows: List<TunnelElement>): DesktopTunnelHierarchy {
    require(info.hierarchy == TunnelHierarchy.Present && info.width > 0u && info.height > 0u)
    val start = checkNotNull(info.hierarchyStartedMs)
    val finish = checkNotNull(info.hierarchyFinishedMs)
    require(info.imageStartedMs <= info.imageFinishedMs && start >= info.imageFinishedMs && finish >= start)
    val age = finish - info.imageFinishedMs
    require(age <= 300uL && info.hierarchyAgeMs == age)
    require(rows.size <= 4096)
    var textUnits = 0L
    val elements = rows.mapIndexed { index, e ->
        require(e.left < e.right && e.top < e.bottom && e.right <= info.width && e.bottom <= info.height)
        require(e.className.length <= 256 && e.resourceId.length <= 1024 && e.description.length <= 4096 && e.text.length <= 800)
        textUnits += e.className.length.toLong() + e.resourceId.length + e.description.length + e.text.length
        require(textUnits <= 128 * 1024)
        CapturedSemanticElement(index.toString(), null, e.description.ifEmpty { e.text }, e.className, null,
            e.resourceId.takeIf { it.isNotEmpty() }, null,
            Rect(e.left.toDouble(), e.top.toDouble(), (e.right - e.left).toDouble(), (e.bottom - e.top).toDouble()),
            e.text, false, false)
    }
    return DesktopTunnelHierarchy((start - info.imageFinishedMs).toInt(), finish - start, elements)
}

private fun createWorkDirectory(projects: Path): Path {
    require(projects.isAbsolute && projects == projects.normalize() && projects.toRealPath() == projects)
    val root = projects.resolve(".android-tunnel-work")
    if (!Files.exists(root, LinkOption.NOFOLLOW_LINKS)) Files.createDirectory(root)
    require(Files.isDirectory(root, LinkOption.NOFOLLOW_LINKS) && root.toRealPath() == root)
    require(Files.list(root).use { it.limit(65).count() } < 64)
    return Files.createTempDirectory(root, "capture-")
}

@Composable internal fun AndroidTunnelPanel(controller:DesktopAndroidTunnel){
    val state by controller.state.collectAsState()
    val scope=rememberCoroutineScope()
    if(!state.open)return
    var adb by remember{mutableStateOf("")};var model by remember{mutableStateOf("")};var transport by remember{mutableStateOf("")}
    var excluded by remember{mutableStateOf(false)}
    fun close(){scope.launch{try{controller.close()}catch(_:Exception){}}}
    DialogWindow(onCloseRequest=::close,visible=excluded,title="Android USB mirror"){
        LaunchedEffect(window){try{excludeCaptureWindows(listOf(window.windowHandle.toULong()));excluded=true}
            catch(error:CancellationException){throw error}catch(_:Exception){close()}}
        Column(Modifier.padding(20.dp).widthIn(min=420.dp),verticalArrangement=Arrangement.spacedBy(10.dp)){
            Text("Android USB mirror",style=MaterialTheme.typography.h6)
            state.message?.let{Text(it)}
            if(!state.connected){
                OutlinedTextField(adb,{adb=it},label={Text("Owner's platform-tools adb.exe")},enabled=!state.busy&&androidTunnelAvailable())
                OutlinedTextField(model,{model=it},label={Text("USB model")},enabled=!state.busy&&androidTunnelAvailable())
                OutlinedTextField(transport,{transport=it},label={Text("USB transport ID")},enabled=!state.busy&&androidTunnelAvailable())
                Button(onClick={transport.toUIntOrNull()?.let{controller.connect(adb,model,it)}},enabled=androidTunnelAvailable()&&!state.busy&&!state.cleanupPending&&transport.toUIntOrNull()!=null){Text("Open view only")}
            }else{
                Text(if(state.mode==TunnelMode.OwnerControl)"Owner control" else "View only")
                Row(horizontalArrangement=Arrangement.spacedBy(8.dp)){
                    Button(controller::capture,enabled=!state.busy&&!state.cleanupPending&&state.target==null){Text("Capture and annotate")}
                    if(state.mode==TunnelMode.OwnerControl)TextButton(controller::revokeControl,enabled=!state.busy&&!state.cleanupPending){Text("Revoke input")}
                    else TextButton(controller::requestControl,enabled=!state.busy&&!state.cleanupPending&&state.target==null){Text("Enable phone input…")}
                }
            }
            state.target?.let{target->
                Text("Allow phone input for ${target.model}, USB transport ${target.transportId}? This confirmation expires in 30 seconds.")
                Row{Button({controller.confirmControl(target)},enabled=!state.busy&&!state.cleanupPending){Text("Allow input")};TextButton({controller.cancelConfirmation(target)},enabled=!state.busy&&!state.cleanupPending){Text("Keep view only")}}
            }
            TextButton(::close,enabled=!state.stopping){Text(if(state.cleanupPending)"Retry close" else "Close mirror")}
        }
    }
}
