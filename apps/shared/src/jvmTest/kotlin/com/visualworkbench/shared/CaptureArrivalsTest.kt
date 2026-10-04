package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import org.junit.Assert.*
import org.junit.Test
import kotlin.coroutines.CoroutineContext

class CaptureArrivalsTest {
    @Test fun acceptedMetadataWaitsForOriginalThenAcknowledgesExactCapture() = runBlocking {
        val f = Fixture()
        try {
            f.owner.bind(f.project, 7, 2uL); f.reply = f.capture(3uL, false)
            f.owner.changed(f.project, 7); f.idle(); assertFalse(checkNotNull(f.owner.state.value).originalVerified)
            f.owner.acknowledge(checkNotNull(f.owner.state.value)); assertNotNull(f.owner.state.value)
            f.reply = f.capture(3uL, true); f.owner.changed(f.project, 7); f.idle()
            val shown = checkNotNull(f.owner.state.value); f.reply = null; f.owner.acknowledge(shown); f.idle()
            assertNull(f.owner.state.value); assertEquals(3uL, f.after.last()); assertEquals(0, f.documentReads)
        } finally { f.close() }
    }
    @Test fun latestRequestsCoalesceWhileExactProducerRemainsOwned() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val gate = CompletableDeferred<Unit>()
        try {
            f.owner.bind(f.project, 1, 0uL); f.reply = f.capture(1uL, true)
            f.hook = { if (f.after.size == 1) { entered.complete(Unit); gate.await() } }
            f.owner.changed(f.project, 1); entered.await()
            repeat(100) { f.reply = f.capture((it + 2).toULong(), true); f.owner.changed(f.project, 1) }
            gate.complete(Unit); f.idle()
            assertEquals(2, f.after.size); assertEquals(101uL, f.owner.state.value?.createdHostSeq)
        } finally { gate.complete(Unit); f.close() }
    }
    @Test fun detachJoinsLateNativeQueryAndForbidsRebindingUntilSettlement() = runBlocking {
        val f = Fixture(); val gate = CompletableDeferred<Unit>()
        try {
            f.owner.bind(f.project, 1, 0uL); f.reply = f.capture(1uL, true)
            f.hook = { withContext(NonCancellable) { gate.await() } }
            f.owner.changed(f.project, 1)
            val closing = async(start = CoroutineStart.UNDISPATCHED) { f.owner.detach() }
            assertFalse(closing.isCompleted)
            try { f.owner.bind(f.project, 2, 0uL); fail("retiring query lost ownership") } catch (_: IllegalStateException) {}
            gate.complete(Unit); withTimeout(3000) { closing.await() }; assertNull(f.owner.state.value)
            f.hook = {}; f.owner.bind(f.project, 2, 1uL); f.owner.changed(f.project, 1); assertEquals(1, f.after.size)
        } finally { gate.complete(Unit); f.close() }
    }
    @Test fun queryRefusalIsVisibleAndDoesNotCreateADocumentOrReadyReceipt() = runBlocking {
        val f = Fixture()
        try {
            f.owner.bind(f.project, 1, 0uL); f.hook = { throw WorkflowFailure(WorkflowFailureKind.Invalid) }
            f.owner.changed(f.project, 1); f.idle()
            assertNull(f.owner.state.value); assertNotNull(f.owner.failure.value); assertEquals(0, f.documentReads)
        } finally { f.close() }
    }
    @Test fun cancellationBeforeDispatchSettlesWithoutRestartingCancelledScope() = runBlocking {
        val dispatcher = Queued(); val f = Fixture(dispatcher)
        f.owner.bind(f.project, 1, 0uL); f.owner.changed(f.project, 1)
        f.scope.cancel(); dispatcher.drain(); f.owner.detach()
        assertTrue(f.after.isEmpty()); assertNull(f.owner.state.value)
    }
    private class Queued : CoroutineDispatcher() {
        val queue = ArrayDeque<Runnable>()
        override fun dispatch(context: CoroutineContext, block: Runnable) { queue.addLast(block) }
        fun drain() { var count = 0; while (queue.isNotEmpty()) { check(++count <= 32); queue.removeFirst().run() } }
    }
    private class Fixture(dispatcher: CoroutineDispatcher = Dispatchers.Unconfined) {
        val scope = CoroutineScope(SupervisorJob() + dispatcher)
        var reply: ReceivedCapture? = null; var hook: suspend () -> Unit = {}; var documentReads = 0
        val after = mutableListOf<ULong>()
        val project = object : WorkbenchProject {
            override val changes = emptyFlow<ProjectChange>()
            override suspend fun info(): ProjectInfo = error("not needed")
            override suspend fun document(documentId: String): DocumentSnapshot { documentReads++; error("notification may not select") }
            override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("unused")
            override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("unused")
            override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("unused")
            override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("unused")
            override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("unused")
            override suspend fun export(options: ExportOptions): ExportResult = error("unused")
            override suspend fun close() = Unit
        }
        val owner = CaptureArrivals(scope) { _, seq -> after += seq; val captured = reply; hook(); captured }
        fun capture(seq: ULong, ready: Boolean) = ReceivedCapture(WorkflowBinding("project", "capture", seq, "hash-$seq"), seq,
            "a".repeat(64), "session", seq, 7u, ready)
        suspend fun idle() { withTimeout(3000) { yield(); yield() } }
        suspend fun close() { owner.detach(); scope.cancel() }
    }
}