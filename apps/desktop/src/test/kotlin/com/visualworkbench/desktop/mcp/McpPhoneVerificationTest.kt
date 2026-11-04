package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Path
import java.util.concurrent.atomic.AtomicReference

/** In-memory owners only: no adb, device, native capture or package compiler. */
class McpPhoneVerificationTest {
    private fun id(value: String) = java.util.UUID.nameUUIDFromBytes(value.toByteArray(Charsets.UTF_8)).toString()
    private val binding = WorkflowBinding(id("project"), id("document"), 7uL, "c".repeat(64))
    private fun info(name: String = "package", source: WorkflowBinding = binding, asset: String = "d".repeat(64)) =
        PackageInfo(id(name), "generic", null, source, asset, "b".repeat(64), "2026-11-04", 1024uL, 0u, emptyList(), true, false)
    private fun refusal(block: () -> Unit): McpRecaptureRefusal =
        try { block(); fail("Recapture source admitted"); error("unreachable") }
        catch (error: McpRecaptureUnavailable) { error.reason }

    @Test fun tunnelOriginKeepsItsKindThroughTicketAndCompletion() {
        val targets = McpVerificationTargets()
        val original = info()
        targets.rememberSource(original.binding, original.sourceAssetId, "phone-route", McpVerificationTargetKind.Tunnel)
        assertTrue(targets.rememberPackage(original))
        assertEquals(McpVerificationTargetKind.Tunnel, targets.kind(original))
        assertFalse(targets.needsBinding(original))
        val ticket = targets.begin(original)
        assertEquals("phone-route", ticket.selector)
        val pair = targets.complete(ticket, info("after", WorkflowBinding(id("after-project"), id("after-document"), 1uL, "e".repeat(64))))
        assertEquals(McpVerificationTargetKind.Tunnel, pair.kind)
    }

    @Test fun phoneAppOriginRefusesUntilTheOwnerBindsALiveRoute() {
        val targets = McpVerificationTargets()
        val original = info()
        assertFalse(targets.rememberPackage(original))
        assertTrue(targets.rememberPhoneSource(original))
        assertTrue(targets.rememberPackage(original))
        assertEquals(McpVerificationTargetKind.Phone, targets.kind(original))
        assertTrue(targets.needsBinding(original))
        assertEquals(McpRecaptureRefusal.PhoneRouteUnbound, refusal { targets.begin(original) })
        targets.bindPhone(original, "usb-route")
        assertFalse(targets.needsBinding(original))
        val ticket = targets.begin(original)
        assertEquals("usb-route", ticket.selector)
        val pair = targets.complete(ticket, info("after", WorkflowBinding(id("after-project"), id("after-document"), 1uL, "e".repeat(64))))
        assertEquals(McpVerificationTargetKind.Phone, pair.kind)
    }

    @Test fun rebindingInvalidatesTicketsAdmittedForTheEarlierRoute() {
        val targets = McpVerificationTargets()
        val original = info()
        targets.rememberPhoneSource(original); targets.rememberPackage(original)
        targets.bindPhone(original, "first-route")
        val stale = targets.begin(original)
        targets.bindPhone(original, "first-route") // Same route: no change.
        targets.checkCurrent(stale)
        targets.bindPhone(original, "second-route")
        assertTrue(runCatching { targets.checkCurrent(stale) }.isFailure)
        assertTrue(runCatching { targets.complete(stale, info("after")) }.isFailure)
        assertEquals("second-route", targets.begin(original).selector)
    }

    @Test fun ownerMayRebindATunnelOriginAfterItsExactSessionWasReplaced() {
        val targets = McpVerificationTargets()
        val original = info()
        targets.rememberSource(original.binding, original.sourceAssetId, "first-session", McpVerificationTargetKind.Tunnel)
        targets.rememberPackage(original)
        targets.bindPhone(original, "first-session") // The exact capturing session: unchanged.
        assertEquals(McpVerificationTargetKind.Tunnel, targets.kind(original))
        val stale = targets.begin(original)
        targets.bindPhone(original, "second-session")
        assertEquals(McpVerificationTargetKind.Phone, targets.kind(original))
        assertTrue(runCatching { targets.checkCurrent(stale) }.isFailure)
        targets.abandon(stale)
        val ticket = targets.begin(original)
        assertEquals("second-session", ticket.selector)
        assertEquals(McpVerificationTargetKind.Phone, ticket.kind)
    }

    @Test fun phoneRegistrationNeverReplacesOrImpersonatesARoutedOrigin() {
        val targets = McpVerificationTargets()
        val window = info()
        targets.rememberCapture(window, "window-route")
        assertFalse(targets.rememberPhoneSource(window))
        assertTrue(targets.rememberPackage(window))
        assertEquals(McpVerificationTargetKind.Window, targets.kind(window))
        assertTrue(runCatching { targets.bindPhone(window, "usb-route") }.isFailure)
        // The same canonical source can not change kind or selector later.
        assertTrue(runCatching { targets.rememberSource(window.binding, window.sourceAssetId, "window-route", McpVerificationTargetKind.Tunnel) }.isFailure)
        assertTrue(runCatching { targets.rememberSource(window.binding, window.sourceAssetId, "other-route") }.isFailure)
        // Phone origins are only ever created unbound, never with a caller selector.
        val other = info("other", asset = "f".repeat(64))
        assertTrue(runCatching { targets.rememberSource(other.binding, other.sourceAssetId, "usb-route", McpVerificationTargetKind.Phone) }.isFailure)
        assertNull(targets.kind(other))
    }

    @Test fun closedTunnelSessionsLeaveTheirOriginsWaitingForARebind() {
        val targets = McpVerificationTargets()
        val routed = info()
        targets.rememberSource(routed.binding, routed.sourceAssetId, "session-a", McpVerificationTargetKind.Tunnel)
        assertTrue(targets.rememberPackage(routed))
        val late = info("late", asset = "f".repeat(64))
        targets.rememberUnboundTunnel(late.binding, late.sourceAssetId)
        assertTrue(targets.rememberPackage(late))
        assertEquals(McpVerificationTargetKind.Tunnel, targets.kind(late))
        assertTrue(targets.needsBinding(late))
        assertEquals(McpRecaptureRefusal.PhoneDisconnected, refusal { targets.begin(late) })
        // A tunnel capture is never re-registered as another kind.
        assertFalse(targets.rememberPhoneSource(late))
        val stale = targets.begin(routed)
        assertEquals(setOf("${routed.packageId}/${routed.target}"), targets.retireSelectors(setOf("session-a", "unrelated")))
        assertTrue(runCatching { targets.checkCurrent(stale) }.isFailure)
        targets.abandon(stale)
        assertEquals(McpVerificationTargetKind.Tunnel, targets.kind(routed))
        assertEquals(McpRecaptureRefusal.PhoneDisconnected, refusal { targets.begin(routed) })
        assertEquals(emptySet<String>(), targets.retireSelectors(setOf("session-a")))
        targets.bindPhone(routed, "session-b")
        assertEquals(McpVerificationTargetKind.Phone, targets.kind(routed))
        assertEquals("session-b", targets.begin(routed).selector)
    }

    @Test fun recaptureLabelsRoundTripThroughTheReceiptNote() {
        assertEquals("", mcpRecaptureNote(McpVerificationTargetKind.Window))
        for (kind in McpVerificationTargetKind.entries) assertEquals(kind, mcpRecaptureKind(mcpRecaptureNote(kind)))
        assertEquals(McpVerificationTargetKind.Window, mcpRecaptureKind("an agent note"))
        assertTrue(mcpRecaptureNote(McpVerificationTargetKind.Phone).contains("owner-attested"))
    }

    @Test fun phoneRoutesServeOnlyVerificationAndRetiredRoutesReportDisconnected() {
        val route = PhoneRoute(PhoneCapture(PhoneProject()))
        val routes = mapOf("phone" to route)
        assertNull(mcpSelectPhoneRoute(routes, "window", verification = false))
        assertNull(mcpSelectPhoneRoute(routes, "window", verification = true))
        assertSame(route, mcpSelectPhoneRoute(routes, "phone", verification = true))
        assertTrue(runCatching { mcpSelectPhoneRoute(routes, "phone", verification = false) }.exceptionOrNull() is McpRefused)
        route.retired = true
        assertEquals(McpRecaptureRefusal.PhoneDisconnected, refusal { mcpSelectPhoneRoute(routes, "phone", verification = true) })
    }

    @Test fun windowFeedbackLoopIsReportedAndOtherGuardFailuresAreUnchanged() {
        assertEquals(McpRecaptureRefusal.FeedbackLoop,
            (mcpGuardFailure(TunnelFailure(TunnelFailureKind.FeedbackLoop)) as? McpRecaptureUnavailable)?.reason)
        for (kind in TunnelFailureKind.entries.filter { it != TunnelFailureKind.FeedbackLoop }) {
            val original = TunnelFailure(kind)
            assertSame(original, mcpGuardFailure(original))
        }
    }

    @Test fun manifestPlatformIsReadOnlyFromTheCheckedCaptureField() {
        fun manifest(capture: String) = """{"package_id":"x","source":{"asset_hash":"a"$capture},"markers":[]}""".toByteArray()
        assertEquals("android", mcpManifestCapturePlatform(manifest(""","capture":{"platform":"android","captured_at":"2026-11-04T00:00:00Z"}""")))
        assertEquals("windows", mcpManifestCapturePlatform(manifest(""","capture":{"platform":"windows","captured_at":"2026-11-04T00:00:00Z"}""")))
        assertNull(mcpManifestCapturePlatform(manifest("")))
        assertNull(mcpManifestCapturePlatform("""{"source":{"capture":{"platform":"android"}}}""".toByteArray())) // No markers frame.
        assertNull(mcpManifestCapturePlatform("""{"markers":[],"markers":[]}""".toByteArray()))
        assertNull(mcpManifestCapturePlatform(byteArrayOf(0x7b, 0xff.toByte(), 0x7d)))
    }

    @Test fun tunnelFailuresMapToHonestAvailabilityReasonsOnly() {
        fun reason(kind: TunnelFailureKind) = (mcpRecaptureFailure(TunnelFailure(kind)) as? McpRecaptureUnavailable)?.reason
        assertEquals(McpRecaptureRefusal.FeedbackLoop, reason(TunnelFailureKind.FeedbackLoop))
        assertEquals(McpRecaptureRefusal.TunnelRuntimeUnavailable, reason(TunnelFailureKind.RuntimeUnavailable))
        for (kind in listOf(TunnelFailureKind.Closed, TunnelFailureKind.RetirementPending, TunnelFailureKind.Stale,
            TunnelFailureKind.AmbiguousDevice, TunnelFailureKind.UnauthorizedDevice))
            assertEquals(McpRecaptureRefusal.PhoneDisconnected, reason(kind))
        for (kind in listOf(TunnelFailureKind.Consent, TunnelFailureKind.Timeout, TunnelFailureKind.Storage, TunnelFailureKind.Cancelled)) {
            val original = TunnelFailure(kind)
            assertSame(original, mcpRecaptureFailure(original))
        }
        // Codes are the fixed allowlist the Node bridge accepts.
        assertEquals(listOf("recapture_source_unavailable", "tunnel_runtime_unavailable", "phone_disconnected",
            "phone_route_unbound", "feedback_loop", "source_busy", "phone_mismatch"), McpRecaptureRefusal.entries.map { it.code })
    }

    @Test fun ownerWorkSendsOnlyTheFixedRefusalCategory() = runBlocking {
        val replies = mutableListOf<Pair<String, Map<String, Any?>?>>()
        val refused = mutableListOf<Pair<String, McpRecaptureRefusal>>()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val coded = McpOwnerWork(scope, { id, result -> synchronized(replies) { replies += id to result } }, { error("reply failed") },
            { id, reason -> synchronized(refused) { refused += id to reason } })
        val plain = McpOwnerWork(scope, { id, result -> synchronized(replies) { replies += id to result } }, { error("reply failed") })
        try {
            assertTrue(coded.start("phone") { throw McpRecaptureUnavailable(McpRecaptureRefusal.PhoneDisconnected) })
            assertTrue(coded.start("other") { throw McpRefused() })
            assertTrue(coded.start("ok") { mapOf("ok" to true) })
            assertTrue(plain.start("legacy") { throw McpRecaptureUnavailable(McpRecaptureRefusal.TunnelRuntimeUnavailable) })
            withTimeout(5000) { while (coded.active + plain.active > 0) delay(5) }
            assertEquals(listOf("phone" to McpRecaptureRefusal.PhoneDisconnected), refused.toList())
            assertEquals(setOf("other" to null, "ok" to mapOf("ok" to true), "legacy" to null), replies.toSet())
        } finally { coded.seal().forEach { it.cancel() }; plain.seal().forEach { it.cancel() }; scope.cancel() }
    }

    @Test fun phoneCompileAttachesUiautomatorHierarchyAsAndroidAccessibilityInCapturePixels() = runBlocking {
        val f = PhoneFixture(TunnelHierarchy.Present)
        val result = f.run()
        assertSame(f.compiled, result.compiled)
        assertEquals(f.work.resolve("project.vwb"), result.project)
        assertEquals(1, f.route.captures); assertEquals(1, f.capture.closes)
        assertEquals("", f.capture.options!!.sourcePath)
        assertEquals(f.work.toString(), f.capture.options!!.workDirectory)
        assertEquals(128uL * 1024uL * 1024uL, f.capture.options!!.memoryBudgetBytes)
        assertEquals(SemanticPlatform.AndroidAx, f.semantics.platform)
        assertEquals(SemanticBoundsSpace.CapturePixels, f.semantics.bounds)
        assertEquals(Rect(3.0, 4.0, 10.0, 20.0), f.semantics.elements.single().bounds)
        assertEquals("resource", f.semantics.elements.single().resourceId)
        assertEquals(1, f.semantics.plan.commits); assertEquals(1, f.semantics.collectionsClosed)
        val options = checkNotNull(f.options.get())
        assertEquals(f.semantics.snapshot, options.semanticSnapshotId)
        assertEquals(PackageTarget.Generic(), options.target)
        assertEquals(0, f.unavailable); assertEquals(1, f.retained.size); assertEquals(1, f.settles)
        assertEquals(0, f.compiled.closes)
    }

    @Test fun phoneCompileWithoutHierarchyIsAnHonestScreenshotOnlyPackage() = runBlocking {
        for (hierarchy in listOf(TunnelHierarchy.Missing, TunnelHierarchy.Stale)) {
            val f = PhoneFixture(hierarchy)
            f.run()
            assertEquals(0, f.semantics.begins); assertEquals(1, f.unavailable)
            assertNull(checkNotNull(f.options.get()).semanticSnapshotId)
        }
        val invalid = PhoneFixture(TunnelHierarchy.Present).apply { capture.rows = listOf(phoneElement().copy(left = 20u, right = 1u)) }
        invalid.run()
        assertEquals(0, invalid.semantics.begins); assertEquals(1, invalid.unavailable)
        assertNull(checkNotNull(invalid.options.get()).semanticSnapshotId)
    }

    @Test fun unavailableOrBusyRouteRefusesBeforeAnyPhoneCaptureOrWorkDirectory() = runBlocking {
        for (reason in listOf(McpRecaptureRefusal.TunnelRuntimeUnavailable, McpRecaptureRefusal.PhoneDisconnected)) {
            val f = PhoneFixture(TunnelHierarchy.Missing).apply { route.refusal = reason }
            val actual = try { f.run(); null } catch (error: McpRecaptureUnavailable) { error.reason }
            assertEquals(reason, actual)
            assertEquals(0, f.route.captures); assertNull(f.capture.options); assertTrue(f.retained.isEmpty())
            assertEquals(0, f.workCreated)
        }
        val busy = PhoneFixture(TunnelHierarchy.Missing).apply { route.busy = true }
        assertEquals(McpRecaptureRefusal.SourceBusy, try { busy.run(); null } catch (error: McpRecaptureUnavailable) { error.reason })
        assertEquals(0, busy.route.captures); assertEquals(0, busy.workCreated)
    }

    @Test fun workDirectoryIsCreatedOnlyAfterAnActualScreenshot() = runBlocking {
        val f = PhoneFixture(TunnelHierarchy.Missing)
        f.run()
        assertEquals(1, f.workCreated)
        val refusedInside = PhoneFixture(TunnelHierarchy.Missing)
        var checks = 0
        refusedInside.grant = { if (++checks == 2) throw McpRefused() } // Grant lost after admission, before info().
        assertTrue(runCatching { refusedInside.run() }.isFailure)
        assertEquals(0, refusedInside.workCreated)
        assertEquals(1, refusedInside.capture.closes)
    }

    @Test fun grantLostAfterCompileClosesTheCompiledPackageAndKeepsProjectRetained() = runBlocking {
        val f = PhoneFixture(TunnelHierarchy.Missing)
        var checks = 0
        f.grant = { if (++checks >= 5) throw McpRefused() }
        assertTrue(runCatching { f.run() }.isFailure)
        assertEquals(1, f.compiled.closes)
        assertEquals(1, f.capture.closes)
        assertEquals(1, f.retained.size) // The engine's retirement owner closes it.
    }
}

private fun phoneInfo(hierarchy: TunnelHierarchy) = TunnelCaptureInfo(
    "tunnel-session", 7uL, 11uL, 100u, 80u, 64uL, "a".repeat(64), 990uL, 1000uL, 1000000uL,
    hierarchy, if (hierarchy == TunnelHierarchy.Present) 90uL else null,
    if (hierarchy == TunnelHierarchy.Present) 1020uL else null,
    if (hierarchy == TunnelHierarchy.Present) 1090uL else null)
private fun phoneElement() = TunnelElement(3u, 4u, 13u, 24u, "android.widget.Button", "resource", "Description", "Text")
private val phoneProjectInfo = ProjectInfo("019a3aaa-0123-789a-8abc-000000000001", "MCP phone capture", "fixture-device",
    1uL, false, false, 0uL, "c".repeat(64), listOf("019a3aaa-0123-789a-8abc-000000000002"))

private class PhoneFixture(hierarchy: TunnelHierarchy) {
    val work: Path = Path.of(System.getProperty("java.io.tmpdir")).toAbsolutePath().resolve("vw-phone-fixture-unused")
    val project = PhoneProject()
    val capture = PhoneCapture(project).apply { metadata = phoneInfo(hierarchy); rows = listOf(phoneElement()) }
    val route = PhoneRoute(capture)
    val semantics = PhoneSemantics()
    val compiled = PhoneCompiled()
    val options = AtomicReference<PackageCompileOptions?>(null)
    val retained = mutableListOf<WorkbenchProject>()
    var unavailable = 0
    var settles = 0
    var workCreated = 0
    var grant: suspend () -> Unit = {}
    suspend fun run(): McpPhoneCompiled = compileMcpPhoneCapture(route, { workCreated++; work }, PhoneCore(), "fixture-device",
        McpCaptureProfile(PackageTarget.Generic(), false), { grant() }, { retained += it }, { settles++ }, { unavailable++ },
        semantics = { semantics }, compile = { _, value -> options.set(value); compiled })
}

private class PhoneRoute(private val captured: WorkbenchTunnelCapture) : McpPhoneCaptureRoute {
    var refusal: McpRecaptureRefusal? = null
    var busy = false
    var captures = 0
    override var retired = false
    override val description = "USB phone fixture"
    override fun checkAvailable() { refusal?.let { throw McpRecaptureUnavailable(it) } }
    override fun checkReady() { checkAvailable(); if (busy) throw McpRecaptureUnavailable(McpRecaptureRefusal.SourceBusy) }
    override suspend fun <T> capture(body: suspend (WorkbenchTunnelCapture) -> T): T {
        checkAvailable(); captures++
        try { return body(captured) } finally { captured.close() }
    }
}

private class PhoneCapture(private val project: WorkbenchProject) : WorkbenchTunnelCapture {
    var metadata = phoneInfo(TunnelHierarchy.Missing)
    var rows = emptyList<TunnelElement>()
    var options: CreateFileProject? = null
    var closes = 0
    override fun info() = metadata
    override fun elements() = rows
    override suspend fun createProject(options: CreateFileProject): WorkbenchProject { this.options = options; return project }
    override suspend fun close() { closes++ }
}

private class PhoneCompiled : WorkbenchCompiledPackage {
    var closes = 0
    override val info = PackageInfo("019a3aaa-0123-789a-8abc-000000000003", "generic", null,
        WorkflowBinding(phoneProjectInfo.projectId, phoneProjectInfo.documentIds.single(), 0uL, "c".repeat(64)),
        "a".repeat(64), "b".repeat(64), "2026-11-04", 1uL, 0u, emptyList(), true, false)
    override suspend fun close() { closes++ }
}

private class PhoneProject : WorkbenchProject {
    override val changes = emptyFlow<ProjectChange>()
    override suspend fun info() = phoneProjectInfo
    override suspend fun document(documentId: String): DocumentSnapshot = error("Unused")
    override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("Unused")
    override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("Unused")
    override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("Unused")
    override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("Unused")
    override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("Unused")
    override suspend fun export(options: ExportOptions): ExportResult = error("Unused")
    override suspend fun close() = Unit
}

private class PhoneCore : WorkbenchCore {
    private var next = 0
    override fun newId(unixMs: ULong) = "019a3aaa-0123-789a-8abc-1%011x".format(next++)
    override fun newDeviceId() = "fixture-device"
    override fun cameraMatrix(camera: Camera, inverse: Boolean) = Transform()
    override fun mapPoints(camera: Camera, inverse: Boolean, points: List<Point>) = points
    override suspend fun create(options: CreateProject): WorkbenchProject = error("Unused")
    override suspend fun open(path: String): WorkbenchProject = error("Unused")
    override suspend fun layoutText(text: String, font: String, size: Float): TextLayout = error("Unused")
}

private class PhoneSemantics : WorkbenchSemantics {
    val plan = PhonePlan()
    var begins = 0
    var collectionsClosed = 0
    var platform: SemanticPlatform? = null
    var bounds: SemanticBoundsSpace? = null
    var snapshot: String? = null
    var elements = emptyList<CapturedSemanticElement>()
    override suspend fun beginCapture(binding: WorkflowBinding, memoryBudgetBytes: ULong): WorkbenchSemanticCollection {
        begins++
        return object : WorkbenchSemanticCollection {
            override fun describe() = SemanticCaptureInfo(binding, "tunnel-session", 11uL, 7u, "a".repeat(64), 1000L, 100.0, 80.0)
            override suspend fun prepare(metadata: WorkflowMetadata, snapshotId: String, platform: SemanticPlatform,
                frameDeltaMs: Int, collectionElapsedMs: ULong, boundsSpace: SemanticBoundsSpace,
                elements: List<CapturedSemanticElement>): WorkbenchWorkflowPlan {
                this@PhoneSemantics.platform = platform; bounds = boundsSpace; snapshot = snapshotId
                this@PhoneSemantics.elements = elements
                return plan
            }
            override fun close() { collectionsClosed++ }
        }
    }
    override suspend fun document(binding: WorkflowBinding, snapshotId: String, memoryBudgetBytes: ULong): SemanticDocument = error("Unused")
    override suspend fun snap(binding: WorkflowBinding, snapshotId: String, query: SemanticSnapQuery,
        documentToScreen: Transform, memoryBudgetBytes: ULong): SemanticSnap? = error("Unused")
    override suspend fun export(binding: WorkflowBinding, snapshotId: String, elementEids: List<String>?,
        memoryBudgetBytes: ULong): SemanticProjection = error("Unused")
}

private class PhonePlan : WorkbenchWorkflowPlan {
    var commits = 0
    override fun describe(): WorkflowPlanInfo = error("Unused")
    override suspend fun commit(): WorkflowReceipt { commits++; return WorkflowReceipt("transaction", phoneProjectInfo, false) }
    override fun close() = Unit
}
