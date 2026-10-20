package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Files
import java.nio.file.Path
import java.util.Comparator

/** In-memory native owners only. No library, device, provider or window opens. */
class DesktopAndroidTunnelTest {
    @Test fun unavailablePackageDoesNotTryPathsOrStartNativeWork() = fixture(available = false) { f ->
        f.controller.connect("entered-adb.exe", "entered-model", 17u)
        f.idle()
        assertEquals(0, f.opens)
        assertEquals(0, f.preparations)
        assertFalse(f.controller.state.value.connected)
        assertTrue(f.controller.state.value.message!!.contains("unavailable"))
        f.controller.close()
        assertEquals(0, f.retries)
        assertEquals(0, f.assertions)
    }

    @Test fun cancelledOpenRetainsLateOwnerAndCloseSignalsItBeforeRetirement() = fixture { f ->
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        f.open = { entered.complete(Unit); tunnelTestLateResult(release, f.tunnel) }
        f.controller.connect("adb.exe", "model", 17u)
        entered.await()
        val closing = async(start = CoroutineStart.UNDISPATCHED) { f.controller.close() }
        assertFalse(closing.isCompleted)
        assertTrue(f.controller.state.value.open)
        assertTrue(f.controller.state.value.stopping)
        f.controller.connect("another.exe", "other", 18u)
        release.complete(Unit)
        closing.await()
        assertEquals(1, f.opens)
        assertEquals(0, f.tunnel.states)
        assertEquals(listOf("tunnel:cancel", "tunnel:close"), f.events)
        assertEquals(listOf(true, false), f.modals)
        assertEquals(DesktopTunnelState(), f.controller.state.value)
    }

    @Test fun cancelledScopeKeepsLateAcquisitionUntilAnExternalCloseJoinsIt() = fixture { f ->
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        f.open = { entered.complete(Unit); tunnelTestLateResult(release, f.tunnel) }
        f.controller.connect("adb.exe", "model", 17u)
        entered.await()
        f.scope.cancel()
        release.complete(Unit)
        f.idle()
        assertTrue(f.controller.state.value.cleanupPending)
        assertEquals(0, f.tunnel.states)
        assertEquals(0, f.tunnel.closes)
        f.controller.close()
        assertEquals(1, f.tunnel.closes)
    }

    @Test fun cancelledCaptureRetainsLateChildAndDoesNotReadOrDeliverIt() = fixture { f ->
        f.connect()
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        f.tunnel.captureAction = { entered.complete(Unit); tunnelTestLateResult(release, f.capture) }
        f.controller.capture()
        entered.await()
        val closing = async(start = CoroutineStart.UNDISPATCHED) { f.controller.close() }
        assertFalse(closing.isCompleted)
        release.complete(Unit)
        closing.await()
        assertEquals(0, f.capture.reads)
        assertEquals(1, f.capture.closes)
        assertEquals(0, f.deliveries)
        assertTrue(f.events.indexOf("capture:close") < f.events.indexOf("tunnel:close"))
    }

    @Test fun cancelledImportRetainsLateProjectAndJoinsBeforeCaptureOrParentClose() = fixture { f ->
        f.connect()
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        f.capture.createAction = { entered.complete(Unit); tunnelTestLateResult(release, f.project) }
        f.controller.capture()
        entered.await()
        val closing = async(start = CoroutineStart.UNDISPATCHED) { f.controller.close() }
        assertFalse(closing.isCompleted)
        assertEquals(0, f.capture.closes)
        release.complete(Unit)
        closing.await()
        assertEquals(1, f.project.closes)
        assertEquals(0, f.deliveries)
        assertTrue(f.events.indexOf("project:close") < f.events.indexOf("capture:close"))
        assertTrue(f.events.indexOf("capture:close") < f.events.indexOf("tunnel:close"))
        assertEquals(128uL * 1024uL * 1024uL, f.capture.options!!.memoryBudgetBytes)
        tunnelTestAssertWorkEmpty(f.root)
    }

    @Test fun failedCaptureCloseRetainsParentsAndSealsActionsUntilExplicitRetry() = fixture { f ->
        f.connect()
        f.capture.readFailure = true
        f.capture.closeFailure = true
        f.controller.capture()
        f.idle()
        assertTrue(f.controller.state.value.cleanupPending)
        assertEquals(0, f.tunnel.closes)
        val captureCalls = f.tunnel.captures
        f.controller.capture()
        f.controller.requestControl()
        f.controller.connect("other", "other", 18u)
        f.idle()
        assertEquals(captureCalls, f.tunnel.captures)
        assertEquals(0, f.tunnel.begins)
        assertEquals(1, f.opens)
        f.capture.closeFailure = false
        f.controller.close()
        assertEquals(2, f.capture.closes)
        assertEquals(1, f.tunnel.closes)
        assertFalse(f.controller.state.value.cleanupPending)
    }

    @Test fun ownerlessRetirementPendingCannotBeReplacedAndRetriesNativeRecovery() = fixture { f ->
        f.open = { throw TunnelFailure(TunnelFailureKind.RetirementPending) }
        f.controller.connect("adb.exe", "model", 17u)
        f.idle()
        assertTrue(f.controller.state.value.cleanupPending)
        f.controller.connect("adb.exe", "model", 17u)
        assertEquals(1, f.opens)
        f.retryFailure = true
        assertTrue(runCatching { f.controller.close() }.isFailure)
        assertTrue(f.controller.state.value.open)
        assertEquals(0, f.assertions)
        f.retryFailure = false
        f.controller.close()
        assertEquals(2, f.retries)
        assertEquals(1, f.assertions)
        assertFalse(f.controller.state.value.open)
    }

    @Test fun failedSessionCloseRetainsExactOwnerAndGlobalFenceForRetry() = fixture { f ->
        f.connect()
        f.tunnel.closeFailure = true
        assertTrue(runCatching { f.controller.close() }.isFailure)
        assertTrue(f.controller.state.value.cleanupPending)
        assertEquals(listOf(true), f.modals)
        assertEquals(0, f.retries)
        f.controller.connect("new", "new", 18u)
        assertEquals(1, f.opens)
        f.tunnel.closeFailure = false
        f.controller.close()
        assertEquals(2, f.tunnel.closes)
        assertEquals(1, f.assertions)
    }

    @Test fun cancelledConcurrentClosersBothWaitForActualOwnerSettlement() = fixture { f ->
        f.connect()
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        f.tunnel.closeAction = { entered.complete(Unit); release.await() }
        val first = launch(start = CoroutineStart.UNDISPATCHED) { f.controller.close() }
        entered.await()
        val second = launch(start = CoroutineStart.UNDISPATCHED) { f.controller.close() }
        first.cancel(); second.cancel()
        assertFalse(first.isCompleted)
        assertFalse(second.isCompleted)
        assertTrue(f.controller.state.value.stopping)
        f.controller.connect("other", "other", 18u)
        assertEquals(1, f.opens)
        release.complete(Unit)
        first.join(); second.join()
        assertEquals(1, f.tunnel.closes)
        assertEquals(DesktopTunnelState(), f.controller.state.value)
        f.controller.show()
        assertTrue(f.controller.state.value.open)
    }

    @Test fun onlyTheExactDisplayedConfirmationIsDispatchedOnce() = fixture { f ->
        f.connect()
        val displayed = f.confirmation()
        f.controller.confirmControl(displayed.copy())
        assertSame(displayed, f.controller.state.value.target)
        assertEquals(0, f.tunnel.confirms)
        f.controller.confirmControl(displayed)
        f.controller.confirmControl(displayed)
        f.idle()
        assertEquals(1, f.tunnel.confirms)
        assertSame(displayed, f.tunnel.confirmed)
        assertEquals(TunnelMode.OwnerControl, f.controller.state.value.mode)
        assertNull(f.controller.state.value.target)
    }

    @Test fun staleDisplayedEventCannotAcceptAnEqualReplacementConfirmation() = fixture { f ->
        f.connect()
        val old = f.confirmation()
        f.controller.cancelConfirmation(old.copy())
        assertSame(old, f.controller.state.value.target)
        f.controller.cancelConfirmation(old)
        f.idle()
        assertEquals(1, f.tunnel.cancellations)
        val next = f.confirmation()
        assertEquals(old, next)
        assertNotSame(old, next)
        f.controller.confirmControl(old)
        assertSame(next, f.controller.state.value.target)
        assertEquals(0, f.tunnel.confirms)
        f.controller.confirmControl(next)
        f.idle()
        assertEquals(1, f.tunnel.confirms)
    }

    @Test fun closeDuringConfirmationPreparationPreventsNativeInputDispatch() = fixture { f ->
        f.connect()
        val target = f.confirmation()
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        f.before = { entered.complete(Unit); tunnelTestLateResult(release, Unit) }
        f.controller.confirmControl(target)
        entered.await()
        val closing = async(start = CoroutineStart.UNDISPATCHED) { f.controller.close() }
        release.complete(Unit)
        closing.await()
        assertEquals(0, f.tunnel.confirms)
        assertNull(f.controller.state.value.target)
    }

    @Test fun terminalRevokeRetiresWrapperAndRequiresFreshViewOnlyConnect() = fixture { f ->
        f.connect()
        val target = f.confirmation()
        f.controller.confirmControl(target)
        f.idle()
        f.controller.revokeControl()
        f.idle()
        assertEquals(1, f.tunnel.revokes)
        assertEquals(1, f.tunnel.closes)
        assertFalse(f.controller.state.value.connected)
        assertEquals(TunnelMode.Closed, f.controller.state.value.mode)
        assertEquals(1, f.assertions)
        f.open = { DesktopTunnelTestSession(f.events) }
        f.controller.connect("adb.exe", "model", 17u)
        f.idle()
        assertEquals(2, f.opens)
        assertEquals(TunnelMode.ViewOnly, f.controller.state.value.mode)
    }

    @Test fun deliveryCloseFailureIsRetainedWithoutAnImmediateSecondProjectClose() = fixture { f ->
        f.connect()
        f.project.closeFailure = true
        f.controller.capture()
        f.idle()
        assertEquals(1, f.deliveries)
        assertEquals(1, f.project.closes)
        assertEquals(1, f.capture.closes)
        assertTrue(f.controller.state.value.cleanupPending)
        assertEquals(0, f.tunnel.closes)
        f.project.closeFailure = false
        f.controller.close()
        assertEquals(2, f.project.closes)
        assertEquals(1, f.tunnel.closes)
    }

    @Test fun capturePinsDestinationBeforeAcquisitionAndKeepsPixelsNative() = fixture { f ->
        f.connect()
        val entered = CompletableDeferred<Unit>()
        val release = f.gate()
        f.tunnel.captureAction = { entered.complete(Unit); release.await(); f.capture }
        f.controller.capture()
        entered.await()
        f.destination = "replacement"
        release.complete(Unit)
        f.idle()
        assertEquals(listOf("original"), f.deliveredDestinations)
        assertEquals("", f.capture.options!!.sourcePath)
        assertEquals(128uL * 1024uL * 1024uL, f.capture.options!!.memoryBudgetBytes)
        assertEquals(1, f.project.closes)
        assertEquals(1, f.capture.closes)
        assertTrue(f.events.indexOf("capture:close") < f.events.indexOf("deliver:original"))
        tunnelTestAssertWorkEmpty(f.root)
    }

    @Test fun cancellationAfterWorkDeletionCannotRetainAnAlreadyMissingDirectory() = fixture { f ->
        f.connect()
        f.afterWorkDelete = { f.scope.cancel() }
        f.controller.capture()
        f.idle()
        assertEquals(1, f.workDeletes)
        assertEquals(0, f.deliveries)
        assertEquals(1, f.project.closes)
        assertEquals(1, f.capture.closes)
        tunnelTestAssertWorkEmpty(f.root)
        f.controller.close()
        assertEquals(1, f.workDeletes)
        assertFalse(f.controller.state.value.cleanupPending)
    }

    @Test fun semanticAttachmentKeepsMeasuredTimingAndCapturePixelBounds() = fixture { f ->
        f.connect()
        f.capture.metadata = tunnelTestInfo(TunnelHierarchy.Present)
        f.capture.rows = listOf(tunnelTestElement())
        f.controller.capture()
        f.idle()
        assertEquals(1, f.semantics.begins)
        assertEquals(20, f.semantics.delta)
        assertEquals(70uL, f.semantics.elapsed)
        assertEquals(SemanticBoundsSpace.CapturePixels, f.semantics.bounds)
        assertEquals(Rect(3.0, 4.0, 10.0, 20.0), f.semantics.elements.single().bounds)
        assertEquals(1, f.semantics.plan.commits)
        assertEquals(1, f.semantics.plan.closes)
        assertEquals(1, f.semantics.collectionsClosed)
        assertNull(f.warning)
        assertEquals(1, f.deliveries)
    }

    @Test fun refusedSemanticChildCloseKeepsProjectCaptureAndSessionUntilRetry() = fixture { f ->
        f.connect()
        f.capture.metadata = tunnelTestInfo(TunnelHierarchy.Present)
        f.capture.rows = listOf(tunnelTestElement())
        f.semantics.plan.closeFailure = true
        f.controller.capture()
        f.idle()
        assertTrue(f.controller.state.value.cleanupPending)
        assertEquals(0, f.semantics.collectionsClosed)
        assertEquals(0, f.project.closes)
        assertEquals(0, f.capture.closes)
        assertEquals(0, f.deliveries)
        assertTrue(runCatching { f.controller.close() }.isFailure)
        assertEquals(0, f.tunnel.closes)
        f.semantics.plan.closeFailure = false
        f.controller.close()
        assertEquals(1, f.semantics.collectionsClosed)
        assertEquals(1, f.project.closes)
        assertEquals(1, f.capture.closes)
        assertEquals(1, f.tunnel.closes)
    }

    @Test fun invalidHierarchyRemainsAnHonestScreenshotOnlyCapture() = fixture { f ->
        f.connect()
        f.capture.metadata = tunnelTestInfo(TunnelHierarchy.Present)
        f.capture.rows = listOf(tunnelTestElement().copy(left = 20u, right = 1u))
        f.controller.capture()
        f.idle()
        assertEquals(0, f.semantics.begins)
        assertEquals(1, f.deliveries)
        assertTrue(f.warning!!.contains("could not be attached"))
        assertFalse(f.controller.state.value.cleanupPending)
    }

    @Test fun hierarchyRefusesUnsignedOverflowOutsidePixelsAndDishonestTiming() {
        val info = tunnelTestInfo(TunnelHierarchy.Present)
        val row = tunnelTestElement()
        val invalidRows = listOf(row.copy(left = UInt.MAX_VALUE), row.copy(right = 101u),
            row.copy(top = 25u, bottom = 4u), row.copy(bottom = 81u), row.copy(text = "x".repeat(801)))
        for (invalid in invalidRows) assertTrue(runCatching { desktopTunnelHierarchy(info, listOf(invalid)) }.isFailure)
        val invalidTimes = listOf(info.copy(imageStartedMs = 1001uL),
            info.copy(hierarchyStartedMs = 999uL), info.copy(hierarchyFinishedMs = 1019uL),
            info.copy(hierarchyAgeMs = 89uL), info.copy(hierarchyFinishedMs = 1301uL, hierarchyAgeMs = 301uL),
            info.copy(hierarchyFinishedMs = ULong.MAX_VALUE, hierarchyAgeMs = ULong.MAX_VALUE))
        for (invalid in invalidTimes) assertTrue(runCatching { desktopTunnelHierarchy(invalid, listOf(row)) }.isFailure)
        assertTrue(runCatching { desktopTunnelHierarchy(info, List(4097) { row }) }.isFailure)
        val valid = desktopTunnelHierarchy(info, listOf(row))
        assertEquals(20, valid.frameDeltaMs)
        assertEquals(70uL, valid.elapsedMs)
    }

    private fun fixture(available: Boolean = true, body: suspend CoroutineScope.(DesktopTunnelTestFixture) -> Unit) = runBlocking {
        withTimeout(5000) {
            val f = DesktopTunnelTestFixture(this, available)
            try { body(f) } finally {
                withContext(NonCancellable) {
                    f.gates.forEach { it.complete(Unit) }
                    f.retryFailure = false
                    f.tunnel.closeFailure = false
                    f.capture.closeFailure = false
                    f.project.closeFailure = false
                    f.semantics.plan.closeFailure = false
                    f.controller.close()
                    f.scope.coroutineContext[Job]!!.cancelAndJoin()
                    Files.walk(f.root).use { paths -> paths.sorted(Comparator.reverseOrder()).forEach(Files::delete) }
                }
            }
        }
    }
}

private class DesktopTunnelTestFixture(parent: CoroutineScope, available: Boolean) {
    val root: Path = Files.createTempDirectory("vw-desktop-tunnel-").toRealPath()
    val scope = CoroutineScope(parent.coroutineContext + SupervisorJob(parent.coroutineContext[Job]))
    val gates = mutableListOf<CompletableDeferred<Unit>>()
    val events = mutableListOf<String>()
    val modals = mutableListOf<Boolean>()
    val tunnel = DesktopTunnelTestSession(events)
    val project = DesktopTunnelTestProject(events)
    val capture = DesktopTunnelTestCapture(project, events)
    val semantics = DesktopTunnelTestSemantics(events)
    var opens = 0
    var preparations = 0
    var retries = 0
    var assertions = 0
    var retryFailure = false
    var deliveries = 0
    var warning: String? = null
    var destination = "original"
    val deliveredDestinations = mutableListOf<String>()
    var before: suspend () -> Unit = {}
    var afterWorkDelete: () -> Unit = {}
    var workDeletes = 0
    var open: suspend (TunnelOptions) -> WorkbenchAndroidTunnel = { tunnel }
    val controller = DesktopAndroidTunnel(scope, DesktopTunnelTestCore(), "fixture-device", root, root.resolve("runtime"),
        beforeOpen = { preparations++; before() }, route = {
            val selected = destination
            val deliver: suspend (WorkbenchProject, Path, String?) -> Unit = { captured, _, message ->
                events += "deliver:$selected"
                deliveries++
                deliveredDestinations += selected
                warning = message
                captured.close()
            }
            deliver
        }, modal = { modals += it }, mayOpen = { true }, report = {}, factories = DesktopTunnelFactories(
            available = { available }, open = { opens++; open(it) },
            retryRetirement = { retries++; if (retryFailure) error("native retirement retained") },
            assertRetired = { assertions++ }, semantics = { semantics },
            deleteWork = { Files.delete(it); workDeletes++; afterWorkDelete() }))
    init { tunnel.captureAction = { capture }; controller.show() }
    fun gate() = CompletableDeferred<Unit>().also { gates += it }
    suspend fun idle() { controller.state.first { !it.busy } }
    suspend fun connect() { controller.connect("adb.exe", "model", 17u); idle(); assertTrue(controller.state.value.connected) }
    suspend fun confirmation(): TunnelTarget { controller.requestControl(); idle(); return checkNotNull(controller.state.value.target) }
}

private suspend fun <T> tunnelTestLateResult(release: CompletableDeferred<Unit>, result: T): T {
    try { release.await() }
    catch (_: CancellationException) { withContext(NonCancellable) { release.await() } }
    return result
}

private fun tunnelTestAssertWorkEmpty(root: Path) {
    val work = root.resolve(".android-tunnel-work")
    assertEquals(0L, Files.list(work).use { it.count() })
}

private fun tunnelTestInfo(hierarchy: TunnelHierarchy = TunnelHierarchy.Missing) = TunnelCaptureInfo(
    "capture-session", 7uL, 11uL, 100u, 80u, 64uL, "fixture-asset", 990uL, 1000uL, 1000000uL,
    hierarchy, if (hierarchy == TunnelHierarchy.Present) 90uL else null,
    if (hierarchy == TunnelHierarchy.Present) 1020uL else null,
    if (hierarchy == TunnelHierarchy.Present) 1090uL else null)
private fun tunnelTestElement() = TunnelElement(3u, 4u, 13u, 24u, "Button", "resource", "Description", "Text")
private val tunnelTestProjectInfo = ProjectInfo("project", "Capture", "fixture-device", 1uL, false, false, 0uL, "hash", listOf("document"))

private class DesktopTunnelTestSession(private val events: MutableList<String>) : WorkbenchAndroidTunnel {
    var states = 0
    var closes = 0
    var captures = 0
    var begins = 0
    var confirms = 0
    var cancellations = 0
    var revokes = 0
    var closeFailure = false
    var confirmed: TunnelTarget? = null
    var captureAction: suspend () -> WorkbenchTunnelCapture = { error("Unused") }
    var closeAction: suspend () -> Unit = {}
    override suspend fun state(): TunnelState { states++; return TunnelState(TunnelMode.ViewOnly, 7uL) }
    override suspend fun beginControlConfirmation(): TunnelTarget { begins++; return TunnelTarget("model", 17u, 7uL) }
    override suspend fun confirmControl(target: TunnelTarget): TunnelState {
        confirms++; confirmed = target; return TunnelState(TunnelMode.OwnerControl, 7uL)
    }
    override suspend fun cancelControlConfirmation() { cancellations++ }
    override suspend fun revokeControl(): TunnelState { revokes++; return TunnelState(TunnelMode.Closed, 0uL) }
    override suspend fun capture(): WorkbenchTunnelCapture { captures++; return captureAction() }
    override fun cancel() { events += "tunnel:cancel" }
    override suspend fun close() {
        closes++; events += "tunnel:close"; closeAction()
        if (closeFailure) error("session retained")
    }
}

private class DesktopTunnelTestCapture(private val project: WorkbenchProject, private val events: MutableList<String>) : WorkbenchTunnelCapture {
    var reads = 0
    var closes = 0
    var readFailure = false
    var closeFailure = false
    var metadata = tunnelTestInfo()
    var rows = emptyList<TunnelElement>()
    var options: CreateFileProject? = null
    var createAction: suspend () -> WorkbenchProject = { project }
    override fun info(): TunnelCaptureInfo { reads++; if (readFailure) error("read refused"); return metadata }
    override fun elements() = rows
    override suspend fun createProject(options: CreateFileProject): WorkbenchProject {
        this.options = options; events += "project:import"; return createAction()
    }
    override suspend fun close() { closes++; events += "capture:close"; if (closeFailure) error("capture retained") }
}

private class DesktopTunnelTestProject(private val events: MutableList<String>) : WorkbenchProject {
    var closes = 0
    var closeFailure = false
    override val changes = emptyFlow<ProjectChange>()
    override suspend fun info() = tunnelTestProjectInfo
    override suspend fun document(documentId: String): DocumentSnapshot = error("Unused")
    override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("Unused")
    override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("Unused")
    override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("Unused")
    override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("Unused")
    override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("Unused")
    override suspend fun export(options: ExportOptions): ExportResult = error("Unused")
    override suspend fun close() { closes++; events += "project:close"; if (closeFailure) error("project retained") }
}

private class DesktopTunnelTestCore : WorkbenchCore {
    private var next = 0
    override fun newId(unixMs: ULong) = "fixture-${next++}"
    override fun newDeviceId() = "fixture-device"
    override fun cameraMatrix(camera: Camera, inverse: Boolean) = Transform()
    override fun mapPoints(camera: Camera, inverse: Boolean, points: List<Point>) = points
    override suspend fun create(options: CreateProject): WorkbenchProject = error("Unused")
    override suspend fun open(path: String): WorkbenchProject = error("Unused")
    override suspend fun layoutText(text: String, font: String, size: Float): TextLayout = error("Unused")
}

private class DesktopTunnelTestSemantics(private val events: MutableList<String>) : WorkbenchSemantics {
    val plan = DesktopTunnelTestPlan(events)
    var begins = 0
    var collectionsClosed = 0
    var delta: Int? = null
    var elapsed: ULong? = null
    var bounds: SemanticBoundsSpace? = null
    var elements = emptyList<CapturedSemanticElement>()
    override suspend fun beginCapture(binding: WorkflowBinding, memoryBudgetBytes: ULong): WorkbenchSemanticCollection {
        begins++
        return object : WorkbenchSemanticCollection {
            override fun describe() = SemanticCaptureInfo(binding, "capture-session", 11uL, 7u, "fixture-asset", 1000L, 100.0, 80.0)
            override suspend fun prepare(metadata: WorkflowMetadata, snapshotId: String, platform: SemanticPlatform,
                frameDeltaMs: Int, collectionElapsedMs: ULong, boundsSpace: SemanticBoundsSpace,
                elements: List<CapturedSemanticElement>): WorkbenchWorkflowPlan {
                delta = frameDeltaMs; elapsed = collectionElapsedMs; bounds = boundsSpace
                this@DesktopTunnelTestSemantics.elements = elements
                return plan
            }
            override fun close() { collectionsClosed++; events += "collection:close" }
        }
    }
    override suspend fun document(binding: WorkflowBinding, snapshotId: String, memoryBudgetBytes: ULong): SemanticDocument = error("Unused")
    override suspend fun snap(binding: WorkflowBinding, snapshotId: String, query: SemanticSnapQuery,
        documentToScreen: Transform, memoryBudgetBytes: ULong): SemanticSnap? = error("Unused")
    override suspend fun export(binding: WorkflowBinding, snapshotId: String, elementEids: List<String>?,
        memoryBudgetBytes: ULong): SemanticProjection = error("Unused")
}

private class DesktopTunnelTestPlan(private val events: MutableList<String>) : WorkbenchWorkflowPlan {
    var commits = 0
    var closes = 0
    var closeFailure = false
    override fun describe(): WorkflowPlanInfo = error("Unused")
    override suspend fun commit(): WorkflowReceipt { commits++; return WorkflowReceipt("transaction", tunnelTestProjectInfo, false) }
    override fun close() { closes++; events += "plan:close"; if (closeFailure) error("plan retained") }
}
