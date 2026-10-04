@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.sync.Mutex
import org.junit.Assert.*
import org.junit.Test

/** Runs the actual shared app controller and instruction editor together. All
 * authority/native acquisition boundaries are injected; no fake transcript or
 * network timing is represented as device acceptance. */
class InstructionFocusControllerTest {
    @Test fun explicitShareUsesTheSelectedLiveMarkerAndCurrentVisibleRevision() = runBlocking {
        val f = Fixture()
        try {
            f.start(); assertFalse(f.focus.state.value.sharing); assertFalse(f.focus.state.value.following)
            f.focus.localSelection("a"); yield(); assertTrue(f.peer.sent.isEmpty())
            f.focus.sharing(true); f.waitFor { f.peer.sent.size == 1 }
            assertEquals(f.attachment!!.binding to "a", f.peer.sent.single())
            f.info = f.info.copy(stateHash = "optimistic-same-host-sequence"); f.publish()
            f.focus.revisionChanged(); f.waitFor { f.peer.sent.size == 2 }
            assertEquals(f.attachment!!.binding, f.peer.sent.last().first)
            f.focus.localSelection("not-a-marker"); f.waitFor { f.peer.sent.size == 3 }
            assertNull(f.peer.sent.last().second)
        } finally { f.close() }
    }
    @Test fun peerFieldOpensWithoutEchoOrAnyCanonicalEdit() = runBlocking {
        val f = Fixture()
        try {
            f.start(); f.focus.sharing(true); f.waitFor { f.peer.sent.isNotEmpty() }
            f.peer.sent.clear(); f.focus.following(true)
            f.waitFor { f.selected.any { it == "b" to true } }
            assertEquals("instruction-b", f.editor.state.value.field!!.instructionId)
            assertEquals("saved b", f.editor.state.value.field!!.text)
            assertTrue(f.peer.sent.isEmpty()); assertEquals(0, f.edits)
            assertTrue(f.focus.state.value.following)
        } finally { f.close() }
    }
    @Test fun enablingShareAfterAttachUsesActualSelectionInsteadOfAnOlderDirtyField() = runBlocking {
        val f = Fixture()
        try {
            f.editor.focusObject("a"); f.idle(); f.editor.update("retained field a")
            f.actualSelection = "b"; f.start(); f.focus.sharing(true)
            f.waitFor { f.peer.sent.isNotEmpty() }
            assertEquals("b", f.peer.sent.single().second)
            assertEquals("retained field a", f.editor.state.value.field!!.text)
        } finally { f.close() }
    }
    @Test fun dirtyDraftSurvivesPeerFocusAndExplicitDiscardAllowsLatestSelection() = runBlocking {
        val f = Fixture()
        try {
            f.start(); f.editor.focusObject("a"); f.idle()
            f.editor.update("every unsaved literal word")
            f.focus.following(true); f.waitFor { f.peer.queries > 0 }
            assertEquals("instruction-a", f.editor.state.value.field!!.instructionId)
            assertEquals("every unsaved literal word", f.editor.state.value.field!!.text)
            assertTrue(f.selected.none { it.second })
            f.editor.discard(); f.idle(); f.waitFor { f.selected.any { it == "b" to true } }
            assertEquals("instruction-b", f.editor.state.value.field!!.instructionId)
        } finally { f.close() }
    }
    @Test fun opsWhileRemoteDraftIsAcquiringCannotPublishItsOldField() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.acquire = { if (it == "instruction-b") withContext(NonCancellable) { entered.complete(Unit); release.await() } }
        try {
            f.start(); f.focus.following(true); withTimeout(3000) { entered.await() }
            f.info = f.info.copy(stateHash = "new-visible"); f.publish(); f.focus.revisionChanged()
            release.complete(Unit); f.idle()
            assertTrue(f.selected.none { it.second }); assertNull(f.editor.state.value.field)
            assertEquals(1, f.draftsClosed)
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun disablingFollowWhileNativeFieldArrivesClosesItsHandleWithoutPublishing() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.acquire = { withContext(NonCancellable) { entered.complete(Unit); release.await() } }
        try {
            f.start(); f.focus.following(true); withTimeout(3000) { entered.await() }
            f.focus.following(false); release.complete(Unit); f.idle()
            assertNull(f.editor.state.value.field); assertTrue(f.selected.isEmpty()); assertEquals(1, f.draftsClosed)
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun localSelectionWhileRemoteFieldArrivesWinsWithoutPublishingThePeerField() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.acquire = { withContext(NonCancellable) { entered.complete(Unit); release.await() } }
        try {
            f.start(); f.focus.following(true); withTimeout(3000) { entered.await() }
            f.focus.localSelection("a")
            release.complete(Unit); f.idle()
            assertNull(f.editor.state.value.field); assertTrue(f.selected.isEmpty())
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun reconnectRetiresBothChoicesAndOldQueryCannotOpenAnInstruction() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.peer.query = { withContext(NonCancellable) { entered.complete(Unit); release.await() } }
        try {
            f.start(); f.focus.sharing(true); f.focus.following(true); withTimeout(3000) { entered.await() }
            f.peer.emit(FocusSignal(2uL, 2uL, false)); yield()
            f.peer.emit(FocusSignal(3uL, 3uL, true)); release.complete(Unit); yield()
            assertFalse(f.focus.state.value.sharing); assertFalse(f.focus.state.value.following)
            assertNull(f.editor.state.value.field); assertTrue(f.selected.isEmpty())
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun sendFailureIsVisibleAndDoesNotKillTheNextExplicitSelectionProducer() = runBlocking {
        val f = Fixture(); f.peer.failSend = true
        try {
            f.start(); f.focus.localSelection("a"); f.focus.sharing(true)
            f.waitFor { f.focus.state.value.message != null }
            f.peer.failSend = false; f.focus.localSelection("b"); f.waitFor { f.peer.sent.isNotEmpty() }
            assertEquals("b", f.peer.sent.single().second)
        } finally { f.close() }
    }
    @Test fun coalescedReconnectWithoutAnOfflineFlowEmissionStillDisablesBothChoices() = runBlocking {
        val f = Fixture()
        try {
            f.start(); f.focus.sharing(true); f.focus.following(true); f.idle()
            f.peer.emit(FocusSignal(3uL, 3uL, true)); yield()
            assertTrue(f.focus.state.value.available)
            assertFalse(f.focus.state.value.sharing); assertFalse(f.focus.state.value.following)
        } finally { f.close() }
    }
    @Test fun detachJoinsPendingSendBeforeBorrowedLinkMayClose() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.peer.sending = { withContext(NonCancellable) { entered.complete(Unit); release.await() } }
        try {
            f.start(); f.focus.localSelection("a"); f.focus.sharing(true); withTimeout(3000) { entered.await() }
            val close = async(start = CoroutineStart.UNDISPATCHED) { f.focus.detach() }
            assertFalse(close.isCompleted); assertFalse(f.focus.state.value.available)
            release.complete(Unit); withTimeout(3000) { close.await() }
            assertEquals(0, f.peer.sendsActive); assertFalse(f.link.closed)
            f.link.close(); assertTrue(f.link.closed)
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun sameRevisionModalRefusalDoesNotDiscardOrAutomaticallyEditAnything() = runBlocking {
        val f = Fixture(); f.allowed = false
        try {
            f.start(); f.focus.following(true); f.waitFor { f.peer.queries > 0 }
            assertNull(f.editor.state.value.field); assertTrue(f.selected.isEmpty())
            f.allowed = true; f.focus.revisionChanged(); f.waitFor { f.selected.any { it.second } }
            assertEquals(0, f.edits)
        } finally { f.close() }
    }
    @Test fun modalOpenedDuringFieldAcquisitionPreservesSelectionUntilDismissed() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.acquire = { if (it == "instruction-b") withContext(NonCancellable) { entered.complete(Unit); release.await() } }
        try {
            f.editor.focusObject("a"); f.idle(); f.start(); f.focus.following(true)
            withTimeout(3000) { entered.await() }
            f.allowed = false; f.focus.interactionChanged()
            release.complete(Unit); f.idle()
            assertEquals("instruction-a", f.editor.state.value.field!!.instructionId)
            assertEquals("a", f.actualSelection); assertTrue(f.selected.none { it.second })
            f.allowed = true; f.focus.interactionChanged(); f.waitFor { f.selected.any { it.second } }
            assertEquals("instruction-b", f.editor.state.value.field!!.instructionId)
        } finally { release.complete(Unit); f.close() }
    }

    private class Fixture {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val gate = Mutex(); var ids = 0; var edits = 0; var draftsClosed = 0; var allowed = true
        var info = ProjectInfo("project", "fixture", "device", 1uL, false, false, 7uL, "visible", listOf("document"))
        val rows = listOf("a", "b").map { InstructionRow("instruction-$it", listOf(it), InstructionRole.Change,
            "saved $it", InstructionEntryMethod.PcKeyboard, "", 1, false) }
        val project = object : WorkbenchProject {
            override val changes = emptyFlow<ProjectChange>()
            override suspend fun info() = this@Fixture.info
            override suspend fun document(documentId: String) = snapshot()
            override suspend fun close() = Unit
            override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("unused")
            override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("unused")
            override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo { edits++; error("focus must not edit") }
            override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("unused")
            override suspend fun render(documentId: String, rectangle: Rect?) = snapshot().render
            override suspend fun export(options: ExportOptions): ExportResult = error("unused")
        }
        fun snapshot() = DocumentSnapshot("document", "fixture", 64u, 64u, 8u,
            listOf(LayerInfo("layer", "Annotations", true, false, 1.0, "normal")), RenderList(info,
                listOf("a", "b").mapIndexed { index, id -> RenderItem(id, "layer", 1.0, "normal", Rect(1.0, 1.0, 10.0, 10.0),
                    Contours(longArrayOf(), longArrayOf(), uintArrayOf()), Transform(), ObjectStyle(0xff0000ffu, 2.0),
                    Shape.Marker((index + 1).toUInt(), Point(6.0, 6.0), null), false) }))
        var attachment: InstructionAttachment? = InstructionAttachment(project, snapshot())
        fun publish() { attachment = InstructionAttachment(project, snapshot()) }
        var acquire: suspend (String) -> Unit = {}
        val workflows = object : WorkbenchInstructions {
            override suspend fun document(documentId: String, memoryBudgetBytes: ULong) = InstructionDocument(attachment!!.binding, rows, emptyList(), false)
            override suspend fun prepare(binding: WorkflowBinding, metadata: WorkflowMetadata, command: InstructionCommand, memoryBudgetBytes: ULong): WorkbenchWorkflowPlan { edits++; error("focus must not commit") }
            override suspend fun export(binding: WorkflowBinding, memoryBudgetBytes: ULong): InstructionProjection = error("unused")
            override suspend fun beginDraft(binding: WorkflowBinding, instructionId: String, sessionId: String, focusGeneration: ULong, entryMethod: InstructionEntryMethod, memoryBudgetBytes: ULong): WorkbenchInstructionDraft {
                acquire(instructionId)
                return object : WorkbenchInstructionDraft {
                    override fun value() = InstructionDraftValue(sessionId, instructionId, binding, rows.first { it.instructionId == instructionId }.text, entryMethod)
                    override fun update(sessionId: String, focusGeneration: ULong, sequence: ULong, text: String) = DraftUpdateStatus.Applied
                    override suspend fun prepare(sessionId: String, focusGeneration: ULong, metadata: WorkflowMetadata): WorkbenchWorkflowPlan = error("unused")
                    override fun close() { draftsClosed++ }
                }
            }
        }
        val selected = mutableListOf<Pair<String, Boolean>>()
        var actualSelection: String? = null
        val editor = InstructionEditor(scope, gate, { attachment }, { "id-${++ids}" },
            { WorkflowMetadata("txn", "device", 1uL, 1) }, { edits++ }, InstructionEntryMethod.PcKeyboard,
            { workflows }, { localField(it) })
        private fun localField(value: InstructionField) { focus.localField(value) }
        val peer = FakeFocus { attachment!!.binding }
        val link = FakeLink()
        val focus = InstructionFocusController(scope, editor, { attachment }, { allowed },
            { marker, remote -> actualSelection = marker; selected += marker to remote }, { peer }, { actualSelection })
        suspend fun start() { focus.attach(link); waitFor { focus.state.value.available } }
        suspend fun idle() { waitFor { !editor.state.value.busy } }
        suspend fun waitFor(condition: () -> Boolean) { withTimeout(3000) { while (!condition()) delay(1) }; yield() }
        suspend fun close() { focus.detach(); editor.detach(); scope.cancel() }
    }
    private class FakeFocus(val binding: () -> WorkflowBinding) : WorkbenchMarkerFocus {
        override val signals = MutableStateFlow(FocusSignal(1uL, 1uL, true))
        var receivedBinding = binding()
        val sent = mutableListOf<Pair<WorkflowBinding, String?>>()
        var queries = 0; var failSend = false; var sendsActive = 0
        var query: suspend () -> Unit = {}; var sending: suspend () -> Unit = {}
        fun emit(value: FocusSignal) { signals.value = value }
        override fun signal() = signals.value
        override suspend fun send(binding: WorkflowBinding, markerId: String?) {
            sendsActive++
            try { if (failSend) throw WorkflowFailure(WorkflowFailureKind.Stale); sending(); sent += binding to markerId }
            finally { sendsActive-- }
        }
        override suspend fun peer(binding: WorkflowBinding): PeerMarkerFocus? {
            queries++; val reply = PeerMarkerFocus(receivedBinding, "b", "instruction-b", signal().sequence, signal().connectionEpoch)
            query(); return reply.takeIf { it.binding == binding }
        }
    }
    private class FakeLink : ProjectLink {
        var closed = false
        override val endpoints = emptyList<SessionEndpoint>()
        override val changes = emptyFlow<SessionStatus>()
        override fun status(): SessionStatus = error("unused")
        override fun sendViewport(documentId: String, corners: List<Point>): Unit = error("focus must not move camera")
        override fun streamStroke(stroke: WorkbenchStroke, batch: SampleBatch, firstSample: UInt) = Unit
        override fun streamObject(preview: ObjectPreview) = Unit
        override fun streamNewObject(preview: NewObjectPreview) = Unit
        override suspend fun finishPreview(gestureId: String, cancel: Boolean) = Unit
        override suspend fun peerPreviews(documentId: String) = PeerPreviews(0uL, emptyList(), emptyList())
        override suspend fun close() { closed = true }
    }
}
