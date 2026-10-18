package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.sync.Mutex
import org.junit.Assert.*
import org.junit.Test
import java.util.ArrayDeque
import kotlin.coroutines.CoroutineContext

class OfflineReviewInteractionTest {
    private class Queue : CoroutineDispatcher() {
        private val tasks = ArrayDeque<Runnable>()
        override fun dispatch(context: CoroutineContext, block: Runnable) { tasks.addLast(block) }
        fun drain() { var n = 0; while (tasks.isNotEmpty()) { check(n++ < 64); tasks.removeFirst().run() } }
    }
    private class Fixture(dispatcher: CoroutineDispatcher = Dispatchers.Unconfined) {
        val scope = CoroutineScope(SupervisorJob() + dispatcher)
        val gate = Mutex()
        var nativeInfo = ProjectInfo("project", "private", "device", 10uL, false, false, 4uL, "a".repeat(64), listOf("document"))
        var publishedInfo = nativeInfo
        var epoch = 1L
        var admitted = true
        var stageCalls = 0; var submitCalls = 0; var keepCalls = 0; var cancelCalls = 0
        var discardedStageResult = false; var discardedSubmitResult = false; var submitStorageError = false
        var detailStarted: CompletableDeferred<Unit>? = null; var detailRelease: CompletableDeferred<Unit>? = null
        val receipts = mutableListOf<WorkflowReceipt>()
        val value = OfflineReviewValue("\"literal other value\"", 21uL, "c".repeat(64), false, true)
        var detail = OfflineReviewDetails(nativeInfo.offlineReviewBinding(), "conflict", "object", "documents.definition.title", false,
            value, value, value, null, null, null, OfflineReviewProgress.Unreviewed, 10uL)
        val project = object : WorkbenchProject {
            override val changes = emptyFlow<ProjectChange>()
            override suspend fun info(): ProjectInfo = nativeInfo
            override suspend fun document(documentId: String): DocumentSnapshot = error("unused")
            override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("unused")
            override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("unused")
            override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("unused")
            override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("unused")
            override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("unused")
            override suspend fun export(options: ExportOptions): ExportResult = error("unused")
            override suspend fun close(): Unit = Unit
        }
        var context: () -> OfflineReviewAttachment? = { OfflineReviewAttachment(project, publishedInfo, epoch) }
        val api = object : WorkbenchOfflineReview {
            override suspend fun page(afterRow: Long, limit: UInt, memoryBudgetBytes: ULong): OfflineReviewPage =
                OfflineReviewPage(nativeInfo.offlineReviewBinding(), if (afterRow == 0L) listOf(OfflineReviewIndex(1, "conflict")) else emptyList())
            override suspend fun details(conflictId: String, memoryBudgetBytes: ULong): OfflineReviewDetails {
                detailStarted?.complete(Unit)
                detailRelease?.let { withContext(NonCancellable) { it.await() } }
                return detail.copy(binding = nativeInfo.offlineReviewBinding(), nextLamport = nativeInfo.nextLamport)
            }
            override suspend fun stage(conflictId: String, expected: OfflineReviewBinding, metadata: WorkflowMetadata,
                choice: OfflineReviewChoice, memoryBudgetBytes: ULong): OfflineReviewMutation {
                stageCalls++
                check(expected == nativeInfo.offlineReviewBinding())
                nativeInfo = nativeInfo.copy(nextLamport = metadata.firstLamport + 1uL)
                detail = detail.copy(binding = expected, reviewed = expected, operation = value,
                    transactionId = metadata.transactionId, progress = OfflineReviewProgress.Draft, nextLamport = nativeInfo.nextLamport)
                if (discardedStageResult) throw CancellationException("discarded completed DTO")
                return OfflineReviewMutation(detail, nativeInfo)
            }
            override suspend fun submit(conflictId: String, expected: OfflineReviewBinding, memoryBudgetBytes: ULong): OfflineReviewSubmission {
                submitCalls++
                check(expected == nativeInfo.offlineReviewBinding())
                nativeInfo = nativeInfo.copy(stateHash = "b".repeat(64), canUndo = true)
                detail = detail.copy(binding = nativeInfo.offlineReviewBinding(), progress = OfflineReviewProgress.Queued)
                if (discardedSubmitResult) throw CancellationException("discarded completed DTO")
                if (submitStorageError) throw WorkflowFailure(WorkflowFailureKind.Storage)
                return OfflineReviewSubmission(WorkflowReceipt(checkNotNull(detail.transactionId), nativeInfo, false), OfflineReviewProgress.Queued, false)
            }
            override suspend fun keep(conflictId: String, expected: OfflineReviewBinding, memoryBudgetBytes: ULong): OfflineReviewMutation {
                keepCalls++; detail = detail.copy(binding = expected, progress = OfflineReviewProgress.Kept)
                return OfflineReviewMutation(detail, nativeInfo)
            }
            override suspend fun cancelDraft(conflictId: String, expected: OfflineReviewBinding, memoryBudgetBytes: ULong): OfflineReviewMutation {
                cancelCalls++; detail = detail.copy(binding = expected, progress = OfflineReviewProgress.Unreviewed, operation = null, reviewed = null, transactionId = null)
                return OfflineReviewMutation(detail, nativeInfo)
            }
        }
        val interaction = OfflineReviewInteraction(scope, gate, { context() }, { admitted },
            { WorkflowMetadata("exact-saved-transaction", "device", publishedInfo.nextLamport, 1000L) }, { "fresh-copy" },
            { a, info -> if (a.project === project && a.epoch == epoch) publishedInfo = info },
            { a, receipt -> if (a.project === project && a.epoch == epoch) { receipts += receipt; publishedInfo = receipt.revision } }, { api })
        suspend fun idle() { repeat(16) { if (!interaction.state.value.busy) return; yield() }; assertFalse(interaction.state.value.busy) }
        suspend fun select() { interaction.select("conflict"); idle() }
        suspend fun close() { detailRelease?.complete(Unit); interaction.detach(); scope.cancel() }
    }

    @Test fun reading_and_refreshing_a_saved_draft_never_submit_it(): Unit = runBlocking {
        val f = Fixture()
        try {
            f.select(); f.interaction.stageSwap(); f.idle()
            f.interaction.refresh(); f.idle()
            assertEquals(1, f.stageCalls); assertEquals(0, f.submitCalls)
            assertEquals(OfflineReviewProgress.Draft, f.interaction.state.value.selected!!.progress)
            assertEquals(11uL, f.publishedInfo.nextLamport)
            assertTrue(f.receipts.isEmpty())
            f.interaction.submit(); f.idle()
            assertEquals(1, f.submitCalls)
            assertEquals("exact-saved-transaction", f.receipts.single().transactionId)
            assertEquals(OfflineReviewProgress.Queued, f.interaction.state.value.selected!!.progress)
        } finally { f.close() }
    }
    @Test fun cancellation_after_durable_stage_reconciles_counter_and_saved_identity(): Unit = runBlocking {
        val f = Fixture()
        try {
            f.select(); f.discardedStageResult = true
            f.interaction.stageSwap(); f.idle()
            assertEquals(11uL, f.publishedInfo.nextLamport)
            assertEquals(OfflineReviewProgress.Draft, f.interaction.state.value.selected!!.progress)
            assertEquals("exact-saved-transaction", f.interaction.state.value.selected!!.transactionId)
            assertEquals(0, f.submitCalls)
        } finally { f.close() }
    }
    @Test fun ambiguous_submit_completion_reconciles_the_exact_durable_edit_for_cancel_and_storage_error(): Unit = runBlocking {
        for (cancel in listOf(true, false)) {
            val f = Fixture()
            try {
                f.select(); f.interaction.stageSwap(); f.idle()
                f.discardedSubmitResult = cancel; f.submitStorageError = !cancel
                f.interaction.submit(); f.idle()
                assertEquals(1, f.submitCalls)
                assertEquals("exact-saved-transaction", f.receipts.single().transactionId)
                assertTrue(f.receipts.single().duplicate)
                assertEquals(OfflineReviewProgress.Queued, f.interaction.state.value.selected!!.progress)
                assertTrue(f.publishedInfo.canUndo)
            } finally { f.close() }
        }
    }
    @Test fun a_changed_visible_hash_preserves_a_saved_draft_until_explicit_refresh_and_cancel(): Unit = runBlocking {
        val f = Fixture()
        try {
            f.select(); f.interaction.stageSwap(); f.idle()
            f.nativeInfo = f.nativeInfo.copy(stateHash = "d".repeat(64)); f.publishedInfo = f.nativeInfo
            f.interaction.submit(); f.idle()
            assertEquals(0, f.submitCalls)
            assertEquals("exact-saved-transaction", f.interaction.state.value.selected!!.transactionId)
            f.interaction.refresh(); f.idle()
            assertTrue(f.interaction.state.value.selected!!.staleDraft)
            f.interaction.cancelDraft(); f.idle()
            assertEquals(1, f.cancelCalls); assertEquals(0, f.submitCalls)
            assertEquals(OfflineReviewProgress.Unreviewed, f.interaction.state.value.selected!!.progress)
        } finally { f.close() }
    }
    @Test fun incomplete_prepared_preview_refuses_new_submission_without_discarding_it(): Unit = runBlocking {
        val f = Fixture()
        try {
            f.select(); f.interaction.stageSwap(); f.idle()
            f.detail = f.detail.copy(operation = f.value.copy(truncated = true))
            f.interaction.refresh(); f.idle(); f.interaction.submit(); f.idle()
            assertEquals(0, f.submitCalls)
            assertEquals(OfflineReviewProgress.Draft, f.interaction.state.value.selected!!.progress)
        } finally { f.close() }
    }
    @Test fun cancel_before_first_dispatch_retires_busy_without_reading_or_staging(): Unit = runBlocking {
        val queue = Queue(); val f = Fixture(queue)
        try {
            f.interaction.select("conflict"); assertTrue(f.interaction.state.value.busy)
            f.interaction.cancelOperation(); queue.drain()
            assertFalse(f.interaction.state.value.busy); assertNull(f.interaction.state.value.selected)
            f.interaction.select("conflict"); queue.drain()
            assertNotNull(f.interaction.state.value.selected)
            assertEquals(0, f.stageCalls); assertEquals(0, f.submitCalls)
        } finally { f.scope.cancel(); queue.drain(); f.close() }
    }
    @Test fun late_old_attachment_result_is_rejected_and_detach_waits_for_actual_retirement(): Unit = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        try {
            f.detailStarted = entered; f.detailRelease = release
            f.interaction.select("conflict"); withTimeout(1000) { entered.await() }
            val closing = launch(start = CoroutineStart.UNDISPATCHED) { f.interaction.detach() }
            assertFalse(closing.isCompleted)
            f.epoch++
            release.complete(Unit); withTimeout(1000) { closing.join() }
            assertNull(f.interaction.state.value.selected)
            f.interaction.resume(); f.interaction.refresh(); f.idle()
            assertNull(f.interaction.state.value.selected)
            assertEquals(0, f.stageCalls); assertEquals(0, f.submitCalls)
        } finally { release.complete(Unit); f.close() }
    }
}
