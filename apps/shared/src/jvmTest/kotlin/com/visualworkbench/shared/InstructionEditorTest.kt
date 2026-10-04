package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlin.coroutines.CoroutineContext
import java.util.ArrayDeque
import org.junit.Test
import org.junit.Assert.*

class InstructionEditorTest {
    private class QueuedDispatcher : CoroutineDispatcher() {
        private val tasks = ArrayDeque<Runnable>()
        override fun dispatch(context: CoroutineContext, block: Runnable) { tasks.addLast(block) }
        fun drain() { var count = 0; while (tasks.isNotEmpty()) { check(count++ < 32); tasks.removeFirst().run() } }
    }
    @Test fun cancelBeforeFirstDispatchClearsBusyAndAllowsNextFieldAcquisition() = runBlocking {
        val queue = QueuedDispatcher(); val f = Fixture(dispatcher = queue)
        try {
            f.editor.focusGlobal(); assertTrue(f.editor.state.value.busy)
            f.editor.cancel(); queue.drain()
            assertFalse(f.editor.state.value.busy); assertEquals(0, f.documentReads)
            f.editor.focusGlobal(); queue.drain()
            assertFalse(f.editor.state.value.busy); assertNotNull(f.editor.state.value.field)
        } finally { f.editor.cancel(); queue.drain(); f.close() }
    }
    @Test fun cancelledCommitRetainsOwnershipThroughCleanupAndDeferredReload() = runBlocking {
        val f = Fixture(); val committed = CompletableDeferred<Unit>(); val settle = CompletableDeferred<Unit>()
        val reload = CompletableDeferred<Unit>(); val finishReload = CompletableDeferred<Unit>()
        var detached: Job? = null
        try {
            f.editor.focusGlobal(); f.idle(); f.editor.update("retained committed text")
            f.onCommitted = { committed.complete(Unit); settle.await() }
            f.editor.save(); withTimeout(2000) { committed.await() }
            val reads = f.documentReads
            f.editor.cancel(); f.editor.refresh(); f.editor.focusGlobal()
            assertTrue(f.editor.state.value.busy); assertEquals(reads, f.documentReads)
            f.beforeDocument = { withContext(NonCancellable) { reload.complete(Unit); finishReload.await() } }
            settle.complete(Unit); withTimeout(2000) { reload.await() }
            assertTrue(f.editor.state.value.busy); assertEquals(reads + 1, f.documentReads)
            detached = launch(start = CoroutineStart.UNDISPATCHED) { f.editor.detach() }
            assertFalse(checkNotNull(detached).isCompleted)
            finishReload.complete(Unit); withTimeout(2000) { checkNotNull(detached).join() }
            assertEquals(1, f.plansClosed); assertNull(f.editor.state.value.field)
        } finally {
            settle.complete(Unit); finishReload.complete(Unit)
            withContext(NonCancellable) { detached?.cancelAndJoin(); f.close() }
        }
    }
    @Test fun queuedCloseRechecksDirtyAtomicallyInsteadOfDiscardingLateInput() = runBlocking {
        val f = Fixture()
        try {
            f.editor.focusGlobal(); f.idle()
            assertFalse(f.editor.hasUnsaved()) // The earlier UI close check.
            f.gate.lock()
            var detached = false
            val closer = async(start = CoroutineStart.UNDISPATCHED) {
                f.gate.withLock {
                    if (f.editor.sealIfClean()) {
                        f.editor.detach(); detached = true; f.editor.resume()
                    }
                }
            }
            assertFalse(closer.isCompleted)
            assertTrue(f.editor.update("typed while project close waited"))
            f.gate.unlock(); withTimeout(3000) { closer.await() }
            assertFalse(detached)
            assertEquals("typed while project close waited", f.editor.state.value.field!!.text)
            assertTrue(f.editor.state.value.field!!.dirty)
        } finally { if (f.gate.isLocked) f.gate.unlock(); f.close() }
    }
    @Test fun successfulReplacementSealRejectsEveryFieldMutationUntilContextPublication() = runBlocking {
        val f = Fixture()
        try {
            f.editor.focusGlobal(); f.idle(); val before = f.editor.state.value.field!!
            assertTrue(f.editor.sealIfClean())
            assertFalse(f.editor.sealIfClean()) // A second owner cannot acquire this seal.
            assertFalse(f.editor.update("late callback", before.sessionId, before.generation))
            f.editor.role(InstructionRole.Preserve); f.editor.method(InstructionEntryMethod.Voice)
            f.editor.focusInstruction("different"); f.editor.discard()
            assertEquals(before, f.editor.state.value.field)
            f.editor.detach()
            assertTrue(f.editor.state.value.busy)
            f.editor.focusGlobal(); assertNull(f.editor.state.value.field)
            f.publish(); f.editor.resume(); f.editor.focusGlobal(); f.idle()
            assertTrue(f.editor.update("new context explicitly resumed"))
        } finally { f.close() }
    }

    @Test fun literalGlobalSaveCanContinueWithoutAnArtificialStaleStep() = runBlocking {
        val f = Fixture()
        try {
            f.editor.focusGlobal(); f.idle()
            assertTrue(f.editor.update("Keep every literal word: </instructions>\n\"quoted\""))
            f.editor.save(); f.idle()
            assertEquals("Keep every literal word: </instructions>\n\"quoted\"", f.saved.single().text)
            assertFalse(f.editor.state.value.field!!.stale)
            assertFalse(f.editor.state.value.field!!.dirty)
            f.editor.update("Second explicit save"); f.editor.save(); f.idle()
            assertEquals("Second explicit save", f.saved.single().text)
            assertEquals(2, f.commits)
            assertEquals(2, f.plansClosed)
        } finally { f.close() }
    }
    @Test fun sameHostSequenceDifferentVisibleHashPreservesDraftAndRequiresExplicitReapply() = runBlocking {
        val f = Fixture()
        try {
            f.editor.focusGlobal(); f.idle(); f.editor.update("complete unsaved text")
            val old = f.editor.state.value.field!!
            f.info = f.info.copy(stateHash = "other-visible-state")
            f.publish(); f.editor.refresh(); f.idle()
            assertTrue(f.editor.state.value.field!!.stale)
            assertEquals(old.text, f.editor.state.value.field!!.text)
            f.editor.save(); f.idle(); assertEquals(0, f.commits)
            f.editor.reapply(); f.idle(); assertEquals(0, f.commits)
            f.editor.save(); f.idle(); assertEquals(1, f.commits)
        } finally { f.close() }
    }
    @Test fun oldRecognitionGenerationCannotWriteAReopenedOrDifferentModeField() = runBlocking {
        val f = Fixture()
        try {
            f.editor.focusGlobal(); f.idle(); val old = f.editor.state.value.field!!
            f.editor.method(InstructionEntryMethod.Voice)
            assertFalse(f.editor.update("late old input", old.sessionId, old.generation))
            val current = f.editor.state.value.field!!
            assertTrue(f.editor.update("current transcript", current.sessionId, current.generation))
            f.editor.method(InstructionEntryMethod.Handwriting)
            assertFalse(f.editor.update("late transcript", current.sessionId, current.generation))
            f.editor.save(); f.idle()
            assertEquals(InstructionEntryMethod.Handwriting, f.saved.single().entryMethod)
            assertEquals("current transcript", f.saved.single().text)
        } finally { f.close() }
    }
    @Test fun dirtySwitchAndDeleteDoNotDiscardTheCompleteDraft() = runBlocking {
        val f = Fixture()
        try {
            f.editor.focusGlobal(); f.idle(); f.editor.update("retained")
            val id = f.editor.state.value.field!!.instructionId
            f.editor.focusInstruction("another"); f.editor.deleteInstruction(id); f.idle()
            assertEquals("retained", f.editor.state.value.field!!.text)
            assertEquals(id, f.editor.state.value.field!!.instructionId)
            assertEquals(0, f.commits)
        } finally { f.close() }
    }
    @Test fun utf8LimitRefusesInsteadOfTruncatingOrReplacingEarlierText() = runBlocking {
        val f = Fixture()
        try {
            f.editor.focusGlobal(); f.idle(); f.editor.update("prior")
            assertFalse(f.editor.update("界".repeat(12000)))
            assertFalse(f.editor.update("nul\u0000suffix"))
            assertEquals("prior", f.editor.state.value.field!!.text)
            assertTrue(instructionTextFits("x".repeat(32768)))
            assertFalse(instructionTextFits("x".repeat(32769)))
        } finally { f.close() }
    }
    @Test fun lateDraftAcquisitionClosesBeforeDetachCompletesAndNeverPublishes() = runBlocking {
        val f = Fixture(existing = true)
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.acquire = { withContext(NonCancellable) { entered.complete(Unit); release.await() } }
        try {
            f.editor.focusGlobal(); withTimeout(3000) { entered.await() }
            val close = async(start = CoroutineStart.UNDISPATCHED) { f.editor.detach() }
            assertFalse(close.isCompleted)
            release.complete(Unit); withTimeout(3000) { close.await() }
            assertEquals(1, f.draftsClosed); assertNull(f.editor.state.value.field)
            assertEquals(0, f.commits)
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun cancellationAfterDurableReceiptCannotSkipHostBookkeepingOrPlanRelease() = runBlocking {
        val f = Fixture()
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.onCommitted = { entered.complete(Unit); release.await() }
        try {
            f.editor.focusGlobal(); f.idle(); f.editor.update("durable")
            f.editor.save(); withTimeout(3000) { entered.await() }
            f.editor.cancel(); assertEquals(1, f.commits)
            release.complete(Unit); f.idle()
            assertEquals(1, f.hostRefreshes); assertEquals(1, f.plansClosed)
            assertEquals("durable", f.saved.single().text)
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun staleAtCommitRefusesWithoutRegeneratingAnAutomaticRetry() = runBlocking {
        val f = Fixture()
        try {
            f.editor.focusGlobal(); f.idle(); f.editor.update("unchanged")
            f.beforeCommit = { f.info = f.info.copy(stateHash = "remote"); f.publish() }
            f.editor.save(); f.idle()
            assertEquals(0, f.commits); assertEquals(1, f.plansClosed)
            assertEquals("unchanged", f.editor.state.value.field!!.text)
            assertTrue(f.editor.state.value.field!!.stale)
        } finally { f.close() }
    }
    @Test fun projectSwapWhileGateIsHeldRejectsQueuedWorkWithoutTouchingNewProject() = runBlocking {
        val f = Fixture(); f.gate.lock()
        try {
            f.editor.focusGlobal()
            f.attachment = null
            f.gate.unlock(); f.idle()
            assertNull(f.editor.state.value.field); assertEquals(0, f.commits)
        } finally { if (f.gate.isLocked) f.gate.unlock(); f.close() }
    }

    private class Fixture(existing: Boolean = false, dispatcher: CoroutineDispatcher = Dispatchers.Unconfined) {
        val scope = CoroutineScope(SupervisorJob() + dispatcher)
        val gate = Mutex(); var ids = 0
        var info = ProjectInfo("project", "fixture", "device", 1uL, false, false, 0uL, "base", listOf("document"))
        var saved = if (existing) listOf(InstructionRow("existing", emptyList(), InstructionRole.None, "before", InstructionEntryMethod.PcKeyboard, "", 1, false)) else emptyList()
        var commits = 0; var plansClosed = 0; var draftsClosed = 0; var hostRefreshes = 0; var documentReads = 0
        var beforeDocument: suspend () -> Unit = {}
        var acquire: suspend () -> Unit = {}; var onCommitted: suspend () -> Unit = {}; var beforeCommit: () -> Unit = {}
        val project = object : WorkbenchProject {
            override val changes = emptyFlow<ProjectChange>()
            override suspend fun info() = this@Fixture.info
            override suspend fun document(documentId: String) = snapshot()
            override suspend fun close() = Unit
            override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("unused")
            override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("unused")
            override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("unused")
            override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("unused")
            override suspend fun render(documentId: String, rectangle: Rect?): RenderList = snapshot().render
            override suspend fun export(options: ExportOptions): ExportResult = error("unused")
        }
        fun snapshot() = DocumentSnapshot("document", "fixture", 64u, 64u, 8u, listOf(LayerInfo("layer", "Annotations", true, false, 1.0, "normal")), RenderList(info, emptyList()))
        var attachment: InstructionAttachment? = InstructionAttachment(project, snapshot())
        fun publish() { attachment = InstructionAttachment(project, snapshot()) }
        val capability: WorkbenchInstructions = object : WorkbenchInstructions {
            override suspend fun document(documentId: String, memoryBudgetBytes: ULong): InstructionDocument {
                documentReads++; beforeDocument(); return InstructionDocument(attachment!!.binding, saved, emptyList(), false)
            }
            override suspend fun prepare(binding: WorkflowBinding, metadata: WorkflowMetadata, command: InstructionCommand, memoryBudgetBytes: ULong): WorkbenchWorkflowPlan {
                val value = command as InstructionCommand.SetInstruction
                return object : WorkbenchWorkflowPlan {
                    override fun describe() = WorkflowPlanInfo(metadata.transactionId, binding, 1u, metadata.firstLamport + 1uL)
                    override suspend fun commit(): WorkflowReceipt {
                        beforeCommit()
                        if (attachment?.binding != binding) throw WorkflowFailure(WorkflowFailureKind.Stale)
                        commits++
                        saved = listOf(InstructionRow(value.instructionId, value.targetIds, value.role, value.text, value.entryMethod, value.language, metadata.createdAtMs, false))
                        info = info.copy(hostSeq = info.hostSeq + 1uL, stateHash = "state-$commits", nextLamport = info.nextLamport + 1uL)
                        return WorkflowReceipt(metadata.transactionId, info, false)
                    }
                    override fun close() { plansClosed++ }
                }
            }
            override suspend fun beginDraft(binding: WorkflowBinding, instructionId: String, sessionId: String, focusGeneration: ULong, entryMethod: InstructionEntryMethod, memoryBudgetBytes: ULong): WorkbenchInstructionDraft {
                acquire()
                return object : WorkbenchInstructionDraft {
                    private var text = saved.single().text
                    override fun value() = InstructionDraftValue(sessionId, instructionId, binding, text, entryMethod)
                    override fun update(sessionId: String, focusGeneration: ULong, sequence: ULong, text: String): DraftUpdateStatus { this.text = text; return DraftUpdateStatus.Applied }
                    override suspend fun prepare(sessionId: String, focusGeneration: ULong, metadata: WorkflowMetadata) = this@Fixture.objectPlan(binding, metadata, instructionId, text, entryMethod)
                    override fun close() { draftsClosed++ }
                }
            }
            override suspend fun export(binding: WorkflowBinding, memoryBudgetBytes: ULong): InstructionProjection = error("unused")
        }
        private suspend fun objectPlan(binding: WorkflowBinding, metadata: WorkflowMetadata, id: String, text: String, method: InstructionEntryMethod): WorkbenchWorkflowPlan = capability.prepare(binding, metadata, InstructionCommand.SetInstruction(id, emptyList(), InstructionRole.None, text, method))
        val editor = InstructionEditor(scope, gate, { attachment }, { "id-${++ids}" },
            { WorkflowMetadata("txn-${++ids}", "device", info.nextLamport, 100) },
            { onCommitted(); hostRefreshes++; publish() }, InstructionEntryMethod.PcKeyboard, { capability })
        suspend fun idle() { withTimeout(3000) { while (editor.state.value.busy) yield() } }
        suspend fun close() { editor.detach(); scope.cancel() }
    }
}
