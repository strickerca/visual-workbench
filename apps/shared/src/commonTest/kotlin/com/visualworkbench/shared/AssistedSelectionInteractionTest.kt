package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.sync.Mutex
import kotlin.coroutines.CoroutineContext
import kotlin.test.*

/** Synthetic adapters exist only here. No test mask is a production backend. */
class AssistedSelectionInteractionTest {
    @Test fun unavailableNeedsNoBackendOrResourcePoolAndLeavesManualSelectionUsable() = runBlocking {
        val f = AssistedFixture(available = false)
        assertEquals(AssistedSelectionPhase.Unavailable, f.ui.state.value.phase)
        assertFalse(f.ui.prompt(point(1.0)))
        assertEquals(0, f.backend.admissions); assertTrue(f.pool.charges.isEmpty())
        f.manual.tool(SelectionTool.Rectangle)
        assertTrue(f.manual.begin(Point(1.0, 1.0), f.camera))
        assertTrue(f.manual.finish(Point(3.0, 3.0)))
        assertIs<SelectionOperation.Rectangle>(f.api.edits.single().operation)
        f.close()
    }

    @Test fun promptsReuseOneEncoderAndOnlyLatestQueuedPromptSurvives() = runBlocking {
        val f = AssistedFixture(); val gate = CompletableDeferred<Unit>()
        f.backend.proposalGate = gate
        assertTrue(f.ui.prompt(point(1.0)))
        assertEquals(1, f.backend.encodes); assertEquals(1, f.backend.prompts.size)
        f.ui.prompt(point(2.0)); f.ui.prompt(point(3.0))
        assertNull(f.ui.state.value.proposal)
        gate.complete(Unit)
        assertEquals(listOf(1.0, 3.0), f.backend.prompts.map { it.prompt.positive.single().x })
        assertEquals(1, f.backend.admissions); assertEquals(1, f.backend.encodes)
        assertEquals(3uL, f.ui.state.value.proposal?.sequence)
        assertTrue(f.api.edits.isEmpty()); f.close()
    }

    @Test fun promptAndProposalBytesAreOwnedBeforeCallerMutation() = runBlocking {
        val f = AssistedFixture(); val gate = CompletableDeferred<Unit>(); f.backend.encodeGate = gate
        val points = mutableListOf(Point(1.0, 1.0))
        f.ui.prompt(AssistedSelectionPrompt(positive = points)); points.clear(); gate.complete(Unit)
        assertEquals(listOf(Point(1.0, 1.0)), f.backend.prompts.single().prompt.positive)
        val proposal = assertNotNull(f.ui.state.value.proposal)
        f.backend.output.fill(0)
        assertEquals(128, proposal.tile(SelectionRegion(0u, 0u, 1u, 1u)).coverage.single().toUByte().toInt())
        assertTrue(f.ui.accept(proposal, SelectionTarget.New("selection", "mask-layer")))
        val imported = assertIs<SelectionOperation.ImportCoverage>(f.api.edits.single().operation)
        assertEquals(128, imported.coverage[0].toUByte().toInt())
        assertFalse(f.ui.accept(proposal, SelectionTarget.New("other", "mask-layer")))
        f.close()
    }

    @Test fun sourceHashRevisionPageAndEpochEachRejectLateProposals() = runBlocking {
        for (change in 0..4) {
            val f = AssistedFixture(); val gate = CompletableDeferred<Unit>(); f.backend.proposalGate = gate
            f.ui.prompt(point(1.0))
            f.live = when (change) {
                0 -> f.source.copy(document = f.source.document.copy(binding = f.source.binding.copy(sourceAssetId = "b".repeat(64))))
                1 -> f.source.copy(document = f.source.document.copy(binding = f.source.binding.copy(hostSeq = 1uL)))
                2 -> f.source.copy(document = f.source.document.copy(binding = f.source.binding.copy(stateHash = "b".repeat(64))))
                3 -> f.source.copy(page = 1u)
                else -> f.source.copy(attachmentEpoch = 2L)
            }
            gate.complete(Unit)
            assertNull(f.ui.state.value.proposal); assertTrue(f.api.edits.isEmpty())
            assertEquals(AssistedSelectionPhase.Stale, f.ui.state.value.phase); f.close()
        }
    }

    @Test fun cancellingLateEncoderJoinsItsActualOwnerBeforeCloseReturns() = runBlocking {
        val f = AssistedFixture(); val gate = CompletableDeferred<Unit>(); f.backend.encodeGate = gate
        f.ui.prompt(point(1.0))
        val closing = launch(start = CoroutineStart.UNDISPATCHED) { f.ui.close() }
        assertFalse(closing.isCompleted); assertTrue(f.ui.retaining)
        closing.cancel(); assertFalse(closing.isCompleted)
        gate.complete(Unit); closing.join()
        assertEquals(1, f.backend.sessionCloses); assertEquals(1, f.backend.runtimeCloses)
        assertFalse(f.ui.retaining); assertTrue(f.api.edits.isEmpty()); assertTrue(f.pool.charges.all { it.closed })
        f.close()
    }

    @Test fun cancellingLateProposalCannotAdoptOrImportIt() = runBlocking {
        val f = AssistedFixture(); val gate = CompletableDeferred<Unit>(); f.backend.proposalGate = gate
        f.ui.prompt(point(1.0)); f.ui.cancel()
        val closing = launch(start = CoroutineStart.UNDISPATCHED) { f.ui.close() }
        assertFalse(closing.isCompleted); assertNull(f.ui.state.value.proposal)
        gate.complete(Unit); closing.join()
        assertNull(f.ui.state.value.proposal); assertTrue(f.api.edits.isEmpty()); assertFalse(f.ui.retaining)
        f.close()
    }

    @Test fun onlyVerifiedPinsAndActualAdmittedExecutionCanReachEncoder() = runBlocking {
        val mismatch = AssistedFixture(); mismatch.backend.observed = mismatch.backend.pins.copy(runtimeArtifactSha256 = "c".repeat(64))
        mismatch.ui.prompt(point(1.0)); assertEquals(0, mismatch.backend.encodes)
        assertEquals(SelectionFailureKind.Unsupported, mismatch.ui.state.value.selectionFailure?.kind)
        assertEquals(1, mismatch.backend.runtimeCloses); mismatch.close()
        val cpu = AssistedFixture(); cpu.backend.executions = setOf(AssistedSelectionExecution.Cpu)
        cpu.ui.prompt(point(1.0)); assertEquals(AssistedSelectionExecution.Cpu, cpu.ui.state.value.execution)
        assertEquals(1, cpu.backend.encodes); cpu.close()
        val none = AssistedFixture(); none.backend.executions = emptySet()
        none.ui.prompt(point(1.0)); assertEquals(0, none.backend.encodes); none.close()
        val noCpu = AssistedFixture(); noCpu.backend.executions = setOf(AssistedSelectionExecution.Gpu)
        noCpu.ui.prompt(point(1.0)); assertEquals(0, noCpu.backend.encodes); noCpu.close()
    }

    @Test fun settledUnsupportedGpuInitialisationFallsBackToAdmittedCpuExactlyOnce() = runBlocking {
        val f = AssistedFixture()
        f.backend.encodeFailures[AssistedSelectionExecution.Gpu] = SelectionFailure(SelectionFailureKind.Unsupported)
        f.ui.prompt(point(1.0)); f.ui.prompt(point(2.0))
        assertEquals(listOf(AssistedSelectionExecution.Gpu, AssistedSelectionExecution.Cpu), f.backend.encodeAttempts)
        assertEquals(1, f.backend.successfulEncodes); assertEquals(1, f.backend.admissions)
        assertEquals(AssistedSelectionExecution.Cpu, f.ui.state.value.execution)
        assertEquals(2, f.backend.prompts.size); f.close()
        val unavailable = AssistedFixture()
        for (path in listOf(AssistedSelectionExecution.Gpu, AssistedSelectionExecution.Cpu))
            unavailable.backend.encodeFailures[path] = SelectionFailure(SelectionFailureKind.Unsupported)
        unavailable.ui.prompt(point(1.0))
        assertEquals(2, unavailable.backend.encodeAttempts.size); assertEquals(0, unavailable.backend.successfulEncodes)
        assertNull(unavailable.ui.state.value.proposal); assertTrue(unavailable.api.edits.isEmpty()); unavailable.close()
    }

    @Test fun cancelledOrStorageFailureNeverInitialisesAFallbackRuntime() = runBlocking {
        val cancelled = AssistedFixture()
        cancelled.backend.onEncode = { cancelled.ui.cancel() }
        cancelled.backend.encodeFailures[AssistedSelectionExecution.Gpu] = SelectionFailure(SelectionFailureKind.Unsupported)
        cancelled.ui.prompt(point(1.0))
        assertEquals(listOf(AssistedSelectionExecution.Gpu), cancelled.backend.encodeAttempts)
        assertNull(cancelled.ui.state.value.proposal); cancelled.close()
        val failed = AssistedFixture()
        failed.backend.encodeFailures[AssistedSelectionExecution.Gpu] = SelectionFailure(SelectionFailureKind.Storage)
        failed.ui.prompt(point(1.0))
        assertEquals(listOf(AssistedSelectionExecution.Gpu), failed.backend.encodeAttempts)
        assertEquals(SelectionFailureKind.Storage, failed.ui.state.value.selectionFailure?.kind); failed.close()
    }

    @Test fun backendOperationCancellationIsTerminalEvenWhileParentRemainsActive() = runBlocking {
        for (stage in 0..2) {
            val f = AssistedFixture()
            val cancelled = CancellationException("Synthetic backend operation cancellation")
            val gate = CompletableDeferred<Unit>()
            when (stage) {
                0 -> f.backend.onAdmit = { throw cancelled }
                1 -> f.backend.encodeFailures[AssistedSelectionExecution.Gpu] = cancelled
                else -> { f.backend.proposalGate = gate; f.backend.proposalFailure = cancelled }
            }
            assertTrue(f.ui.prompt(point(1.0)))
            if (stage == 2) {
                assertTrue(f.ui.prompt(point(2.0))) // Queued behind the cancelled evaluation.
                gate.complete(Unit)
                assertEquals(1, f.backend.prompts.size)
            }
            assertTrue(f.scope.isActive)
            assertEquals(AssistedSelectionPhase.Closed, f.ui.state.value.phase)
            assertFalse(f.ui.state.value.canPrompt); assertFalse(f.ui.prompt(point(3.0)))
            assertNull(f.ui.state.value.proposal); assertTrue(f.api.edits.isEmpty())
            assertFalse(f.ui.retaining); assertTrue(f.pool.charges.all { it.closed })
            assertEquals(if (stage == 0) 0 else 1, f.backend.runtimeCloses)
            assertEquals(if (stage == 2) 1 else 0, f.backend.sessionCloses)
            assertEquals(if (stage == 0) 0 else 1, f.backend.encodes)
            f.close()
        }
    }

    @Test fun backendAdmissionReentrantCloseSeesPublishedJobAndJoinsLateOwner() = runBlocking {
        val f = AssistedFixture(); val gate = CompletableDeferred<Unit>(); var closing: Job? = null
        f.backend.onAdmit = {
            closing = f.scope.launch(start = CoroutineStart.UNDISPATCHED) { f.ui.close() }
            assertFalse(closing.isCompleted)
            withContext(NonCancellable) { gate.await() }
        }
        f.ui.prompt(point(1.0)); assertFalse(closing!!.isCompleted)
        gate.complete(Unit); closing.join()
        assertEquals(0, f.backend.encodes); assertEquals(1, f.backend.runtimeCloses)
        assertFalse(f.ui.retaining); assertTrue(f.api.edits.isEmpty()); f.close()
    }

    @Test fun metadataReentrantCloseSeesPublishedMutationAndCannotReachNativeImport() = runBlocking {
        val f = AssistedFixture(); var closing: Job? = null
        f.ui.prompt(point(1.0)); val proposal = assertNotNull(f.ui.state.value.proposal)
        f.onMetadata = {
            closing = f.scope.launch(start = CoroutineStart.UNDISPATCHED) { f.ui.close() }
            assertFalse(closing.isCompleted)
        }
        assertTrue(f.ui.accept(proposal, SelectionTarget.New("selection", "mask-layer")))
        closing!!.join(); assertTrue(f.api.edits.isEmpty()); assertEquals(0, f.refreshes)
        assertFalse(f.ui.retaining); f.close()
    }

    @Test fun acceptedJobsCancelledBeforeTheirBodiesRunStillRevokeRequestsAndReleaseCharges() = runBlocking {
        val queue = AssistedQueuedDispatcher(); val unstarted = AssistedFixture(dispatcher = queue)
        unstarted.ui.prompt(point(1.0)); unstarted.scope.cancel(); queue.drain()
        assertEquals(0, unstarted.backend.admissions); assertFalse(unstarted.ui.retaining)
        assertEquals(AssistedSelectionPhase.Closed, unstarted.ui.state.value.phase); unstarted.close()
        val secondQueue = AssistedQueuedDispatcher(); val unstartedAccept = AssistedFixture(dispatcher = secondQueue)
        unstartedAccept.ui.prompt(point(1.0)); secondQueue.drain()
        val proposal = assertNotNull(unstartedAccept.ui.state.value.proposal)
        unstartedAccept.ui.accept(proposal, SelectionTarget.New("selection", "mask-layer"))
        unstartedAccept.scope.cancel(); secondQueue.drain()
        assertTrue(unstartedAccept.api.edits.isEmpty()); assertFalse(unstartedAccept.ui.retaining)
        assertTrue(unstartedAccept.pool.charges.all { it.closed }); unstartedAccept.close()
    }

    @Test fun runtimeAndImportCopiesAreAdmittedBeforeEncoderOrCoverageCopy() = runBlocking {
        val f = AssistedFixture(); f.backend.workingSet = 256uL * 1024uL * 1024uL
        f.ui.prompt(point(1.0)); assertEquals(0, f.backend.encodes); assertTrue(f.pool.charges.isEmpty())
        assertEquals(SelectionFailureKind.Memory, f.ui.state.value.selectionFailure?.kind); f.close()
        val malformed = AssistedFixture(); malformed.backend.output = byteArrayOf(1)
        malformed.ui.prompt(point(1.0)); assertNull(malformed.ui.state.value.proposal)
        assertEquals(SelectionFailureKind.Invalid, malformed.ui.state.value.selectionFailure?.kind)
        assertTrue(malformed.api.edits.isEmpty()); malformed.close()
    }

    @Test fun acceptUsesExistingIdentityAndManualBrushAndMathTargetTheImportedVersion() = runBlocking {
        val f = AssistedFixture(); f.ui.prompt(point(1.0)); val proposal = assertNotNull(f.ui.state.value.proposal)
        assertTrue(f.ui.accept(proposal, SelectionTarget.New("selection", "mask-layer")))
        assertEquals(1, f.api.edits.size); assertEquals(1, f.refreshes)
        val edit = f.api.edits.single()
        assertEquals("transaction-1", edit.transactionId); assertEquals(f.source.binding, edit.binding)
        assertEquals(1uL, edit.lamport); assertEquals("device", edit.deviceId)
        val receipt = assertNotNull(f.ui.state.value.receipt)
        assertSame(receipt, f.delivered.single())
        assertEquals(receipt.snapshot.selection, f.manual.state.value.selection?.selection)
        f.manual.tool(SelectionTool.Paint); f.manual.combine(SelectionCombine.Subtract)
        assertTrue(f.manual.begin(Point(1.0, 1.0), f.camera)); assertTrue(f.manual.finish(Point(2.0, 2.0)))
        val correction = f.api.edits[1]
        assertEquals(SelectionTarget.Existing(receipt.snapshot.selection), correction.target)
        assertEquals(SelectionCombine.Subtract, assertIs<SelectionOperation.Paint>(correction.operation).combine)
        assertTrue(f.manual.refine(SelectionRefinement.Expand))
        assertIs<SelectionOperation.Expand>(f.api.edits[2].operation)
        assertEquals(1, f.backend.encodes); f.close()
    }

    @Test fun cancelledNativeImportSettlesAndRefreshesOnceWithoutRetryOrSecondUndoEntry() = runBlocking {
        val f = AssistedFixture(); val gate = CompletableDeferred<Unit>(); f.api.editGate = gate
        f.ui.prompt(point(1.0)); val proposal = assertNotNull(f.ui.state.value.proposal)
        assertTrue(f.ui.accept(proposal, SelectionTarget.New("selection", "mask-layer")))
        f.ui.cancel()
        val closing = launch(start = CoroutineStart.UNDISPATCHED) { f.ui.close() }
        assertFalse(closing.isCompleted); assertEquals(1, f.api.edits.size); assertEquals(0, f.refreshes)
        gate.complete(Unit); closing.join()
        assertEquals(1, f.refreshes); assertNull(f.delivered.single()); assertEquals(1, f.api.edits.size)
        assertFalse(f.ui.accept(proposal, SelectionTarget.New("other", "mask-layer"))); f.close()
    }

    @Test fun existingSelectionFailureIsPreservedAndConsumedProposalIsNeverRetried() = runBlocking {
        val f = AssistedFixture(); val failure = SelectionFailure(SelectionFailureKind.Memory, 100uL, 10uL)
        f.api.failure = failure; f.ui.prompt(point(1.0)); val proposal = assertNotNull(f.ui.state.value.proposal)
        f.ui.accept(proposal, SelectionTarget.Existing(SelectionVersion("selection", "old-mask", 7uL)), SelectionCombine.Intersect)
        assertSame(failure, f.ui.state.value.selectionFailure); assertEquals(1, f.refreshes)
        assertFalse(f.ui.accept(proposal, SelectionTarget.New("other", "mask-layer")))
        assertEquals(1, f.api.edits.size); f.close()
    }

    @Test fun failedRetirementRetainsHandleAndChargeUntilRealCloseSucceeds() = runBlocking {
        val f = AssistedFixture(); f.ui.prompt(point(1.0)); f.backend.failClose = true; f.ui.cancel()
        assertTrue(runCatching { f.ui.close() }.isFailure); assertTrue(f.ui.retaining)
        assertTrue(f.pool.charges.any { !it.closed }); assertEquals(0, f.backend.runtimeCloses)
        f.backend.failClose = false; f.ui.close(); assertFalse(f.ui.retaining)
        assertTrue(f.pool.charges.all { it.closed }); assertEquals(1, f.backend.runtimeCloses); f.close()
    }

    @Test fun concurrentCloseRetriesSerializeTheSameFailedSessionOwner() = runBlocking {
        val f = AssistedFixture(); f.ui.prompt(point(1.0)); f.backend.failClose = true; f.ui.cancel()
        assertEquals(1, f.backend.sessionCloseAttempts); assertTrue(f.ui.retaining)
        f.backend.failClose = false; val gate = CompletableDeferred<Unit>(); f.backend.closeGate = gate
        val first = launch(start = CoroutineStart.UNDISPATCHED) { f.ui.close() }
        val second = launch(start = CoroutineStart.UNDISPATCHED) { f.ui.close() }
        assertFalse(first.isCompleted); assertFalse(second.isCompleted)
        assertEquals(2, f.backend.sessionCloseAttempts)
        gate.complete(Unit); first.join(); second.join()
        assertEquals(1, f.backend.sessionCloses); assertEquals(1, f.backend.runtimeCloses)
        assertFalse(f.ui.retaining); f.close()
    }

    @Test fun reservationReleaseFailureRetainsBothChargesWithoutClosingRuntimeTwice() = runBlocking {
        val f = AssistedFixture(); f.ui.prompt(point(1.0)); f.pool.failRelease = true; f.ui.cancel()
        assertTrue(f.ui.retaining); assertEquals(AssistedSelectionPhase.Failed, f.ui.state.value.phase)
        assertTrue(runCatching { f.ui.close() }.isFailure); assertEquals(1, f.backend.runtimeCloses)
        assertTrue(f.pool.charges.all { !it.closed })
        f.pool.failRelease = false; f.ui.close()
        assertFalse(f.ui.retaining); assertTrue(f.pool.charges.all { it.closed })
        assertEquals(1, f.backend.runtimeCloses); f.close()
    }

    @Test fun invalidPromptCountsFiniteBoundsAndImportDimensionsRefuseBeforeCopying() = runBlocking {
        val f = AssistedFixture()
        for (prompt in listOf(point(Double.NaN), point(8.0), AssistedSelectionPrompt(box = Rect(0.0, 0.0, Double.POSITIVE_INFINITY, 1.0)),
            AssistedSelectionPrompt(positive = List(65) { Point(1.0, 1.0) }))) {
            assertFailsWith<SelectionFailure> { f.ui.prompt(prompt) }
        }
        assertEquals(0, f.backend.admissions)
        val edit = SelectionEdit(f.source.binding, "t", "device", 1uL, 0,
            SelectionTarget.New("o", "l"), SelectionOperation.ImportCoverage(8u, 8u, ByteArray(63), SelectionCombine.Add))
        assertFailsWith<SelectionFailure> { edit.ownedSelectionRequest() }
        val bytes = ByteArray(64) { 127 }; val owned = edit.copy(operation = SelectionOperation.ImportCoverage(8u, 8u, bytes, SelectionCombine.Add)).ownedSelectionRequest()
        bytes.fill(0); assertEquals(127, assertIs<SelectionOperation.ImportCoverage>(owned.operation).coverage[0].toInt())
        f.close()
    }

    private fun point(x: Double): AssistedSelectionPrompt = AssistedSelectionPrompt(positive = listOf(Point(x, 1.0)))
}

private class AssistedFixture(available: Boolean = true, dispatcher: CoroutineDispatcher = Dispatchers.Unconfined) {
    val scope = CoroutineScope(SupervisorJob() + dispatcher)
    val backend = AssistedFakeBackend(); val pool = AssistedFakeResources()
    val api = AssistedFakeSelections()
    val source = AssistedSelectionSource(SelectionDocument(api.binding(), 8u, 8u, 8u, api.info), 0u, 1L)
    var live = source
    var refreshes = 0
    var onMetadata: suspend () -> Unit = {}
    val delivered = mutableListOf<SelectionReceipt?>()
    private val core = AssistedFakeCore()
    private val project = AssistedFakeProject { api.info }
    val camera = Camera(Point(4.0, 4.0), 1.0, 0.0, 8.0, 8.0)
    val manual = SelectionInteraction(core, scope, { 1000L }, { receipt ->
        receipt?.let { api.info = it.revision }; bindManual()
    }, { api })
    val ui = AssistedSelectionInteraction(scope, source, api, Mutex(),
        { onMetadata(); WorkflowMetadata("transaction-1", "device", 1uL, 1000L) }, { it == live }, { receipt ->
            refreshes++; delivered += receipt
            receipt?.let { api.info = it.revision }; bindManual()
            receipt?.let { manual.useObject(it.snapshot.selection.objectId) }
        }, { pool }, if (available) backend.pins else null, if (available) backend else null)
    init { bindManual(); manual.viewport(camera) }
    private fun bindManual() { manual.bind(project, DocumentSnapshot("document", "Fixture", 8u, 8u, 8u,
        emptyList(), RenderList(api.info, emptyList()), 0u), 1L, api.info.hostSeq) }
    suspend fun close() { ui.close(); manual.detach(); scope.cancel() }
}

private class AssistedFakeBackend : AssistedSelectionBackend {
    val pins = AssistedSelectionPins("a".repeat(64), "b".repeat(64), "test-model-license", "test-runtime-license", "test-runtime")
    var observed = pins
    var executions = setOf(AssistedSelectionExecution.Gpu, AssistedSelectionExecution.Cpu)
    var workingSet = 4uL * 1024uL * 1024uL
    var admissions = 0; var encodes = 0; var runtimeCloses = 0; var sessionCloses = 0
    var sessionCloseAttempts = 0
    var successfulEncodes = 0
    var onAdmit: suspend () -> Unit = {}
    var onEncode: suspend (AssistedSelectionExecution) -> Unit = {}
    val encodeAttempts = mutableListOf<AssistedSelectionExecution>()
    val encodeFailures = mutableMapOf<AssistedSelectionExecution, Exception>()
    var failClose = false
    var encodeGate: CompletableDeferred<Unit>? = null; var proposalGate: CompletableDeferred<Unit>? = null
    var proposalFailure: Exception? = null
    var closeGate: CompletableDeferred<Unit>? = null
    var output = ByteArray(64) { 128.toByte() }
    val prompts = mutableListOf<AssistedSelectionPromptRequest>()
    override suspend fun admit(pins: AssistedSelectionPins): AssistedSelectionRuntime {
        admissions++
        onAdmit()
        return object : AssistedSelectionRuntime {
            override val admittedPins: AssistedSelectionPins get() = observed
            override val executions: Set<AssistedSelectionExecution> get() = this@AssistedFakeBackend.executions
            override val workingSetBytes: ULong get() = workingSet
            override suspend fun encode(source: AssistedSelectionSource, execution: AssistedSelectionExecution): AssistedSelectionSession {
                encodes++; encodeAttempts += execution; onEncode(execution)
                encodeFailures[execution]?.let { throw it }
                withContext(NonCancellable) { encodeGate?.await() }; successfulEncodes++
                return object : AssistedSelectionSession {
                    override val source: AssistedSelectionSource = source
                    override val execution: AssistedSelectionExecution = execution
                    override suspend fun propose(request: AssistedSelectionPromptRequest): AssistedSelectionMask {
                        prompts += request; withContext(NonCancellable) { proposalGate?.await() }
                        proposalFailure?.let { throw it }
                        return AssistedSelectionMask(source, request.sequence, 8u, 8u, output)
                    }
                    override suspend fun close() {
                        sessionCloseAttempts++; closeGate?.await()
                        if (failClose) error("Retirement retained"); sessionCloses++
                    }
                }
            }
            override suspend fun close() { runtimeCloses++ }
        }
    }
}
private class AssistedQueuedDispatcher : CoroutineDispatcher() {
    private val tasks = ArrayDeque<Runnable>()
    override fun dispatch(context: CoroutineContext, block: Runnable) { tasks.addLast(block) }
    fun drain() { while (tasks.isNotEmpty()) tasks.removeFirst().run() }
}
private class AssistedFakeResources : SessionResources {
    class Charge(private val fails: () -> Boolean) : SessionResourceCharge {
        var closed = false
        override fun close() { if (fails()) error("Reservation retained"); closed = true }
    }
    var failRelease = false
    val charges = mutableListOf<Charge>()
    override suspend fun reserveForeign(bytes: ULong): SessionResourceCharge = Charge { failRelease }.also { charges += it }
}
private class AssistedFakeSelections : WorkbenchSelections {
    var info = ProjectInfo("project", "Fixture", "device", 1uL, false, false, 0uL, "a".repeat(64), listOf("document"))
    var version = 0uL
    var failure: SelectionFailure? = null
    var editGate: CompletableDeferred<Unit>? = null
    val edits = mutableListOf<SelectionEdit>()
    fun binding(): SelectionBinding = SelectionBinding("project", "document", "a".repeat(64), info.hostSeq, info.stateHash)
    override suspend fun document(documentId: String): SelectionDocument = SelectionDocument(binding(), 8u, 8u, 8u, info)
    override suspend fun snapshot(binding: SelectionBinding, objectId: String, memoryBudgetBytes: ULong): SelectionSnapshot =
        SelectionSnapshot(binding, SelectionVersion(objectId, "mask-$version", version), "mask-layer", 8u, 8u,
            SelectionRegion(0u, 0u, 8u, 8u), true, true)
    override suspend fun tiles(binding: SelectionBinding, selection: SelectionVersion, regions: List<SelectionRegion>, memoryBudgetBytes: ULong): SelectionTiles =
        SelectionTiles(binding, selection, regions.map { SelectionTile(it, ByteArray((it.width * it.height).toInt())) })
    override suspend fun edit(options: SelectionEdit): SelectionReceipt {
        edits += options.ownedSelectionRequest(); failure?.let { throw it }
        editGate?.let { gate -> withContext(NonCancellable) { gate.await() }; throw CancellationException("Synthetic uncertain commit") }
        version++; info = info.copy(hostSeq = info.hostSeq + 1uL, nextLamport = info.nextLamport + 4uL, stateHash = version.toString().padStart(64, '0'))
        val id = when (val target = options.target) { is SelectionTarget.New -> target.objectId; is SelectionTarget.Existing -> target.selection.objectId }
        return SelectionReceipt(options.transactionId, snapshot(binding(), id, options.memoryBudgetBytes), info)
    }
    override suspend fun exportFile(options: SelectionExportOptions): SelectionExportReceipt = error("Unused")
}
private class AssistedFakeCore : WorkbenchCore {
    var next = 0
    override fun newId(unixMs: ULong): String = "manual-${next++}"
    override fun newDeviceId(): String = "device"
    override fun cameraMatrix(camera: Camera, inverse: Boolean): Transform = Transform()
    override fun mapPoints(camera: Camera, inverse: Boolean, points: List<Point>): List<Point> = points.toList()
    override suspend fun create(options: CreateProject): WorkbenchProject = error("Unused")
    override suspend fun open(path: String): WorkbenchProject = error("Unused")
    override suspend fun layoutText(text: String, font: String, size: Float): TextLayout = error("Unused")
}
private class AssistedFakeProject(private val revision: () -> ProjectInfo) : WorkbenchProject {
    override val changes = emptyFlow<ProjectChange>()
    override suspend fun info(): ProjectInfo = revision()
    override suspend fun document(documentId: String): DocumentSnapshot = error("Unused")
    override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("Unused")
    override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("Unused")
    override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("Unused")
    override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("Unused")
    override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("Unused")
    override suspend fun export(options: ExportOptions): ExportResult = error("Unused")
    override suspend fun close(): Unit = Unit
}
