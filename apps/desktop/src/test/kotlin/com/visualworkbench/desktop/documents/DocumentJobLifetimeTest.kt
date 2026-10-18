package com.visualworkbench.desktop.documents

import com.visualworkbench.shared.*
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class DocumentJobLifetimeTest {
    private class Call : DocumentCall {
        val started = CompletableDeferred<Unit>()
        val cancelled = CompletableDeferred<Unit>()
        val complete = CompletableDeferred<Unit>()
        val retirement = CompletableDeferred<Unit>()
        @Volatile var uncertain = false
        var failureOnExit: DocumentFailureKind? = null
        val retireCount = AtomicInteger()
        override fun cancel() { cancelled.complete(Unit) }
        override suspend fun execute(request: DocumentRequest, source: ByteArray): DocumentReply {
            started.complete(Unit); complete.await()
            failureOnExit?.let { throw DocumentFailure(it) }
            return DocumentReply("a".repeat(64), "b".repeat(64), DocumentMetadata.Info(EditableKind.Svg, 1u, 1.0, 1.0, false, emptyList()), byteArrayOf())
        }
        override suspend fun retire() { retireCount.incrementAndGet(); if (uncertain) throw DocumentFailure(DocumentFailureKind.Isolation); retirement.await() }
    }
    private class Backend(val active: Call) : DocumentBackend {
        override val pcRedaction = true
        var calls = 0
        var closes = 0
        override fun call(): DocumentCall { calls++; return active }
        override suspend fun close() { closes++ }
    }
    private val request = DocumentRequest(DocumentOperation.SvgInfo)
    private suspend fun failure(kind: DocumentFailureKind, block: suspend () -> Unit) {
        try { block(); fail("expected fixed failure") } catch (error: DocumentFailure) { assertEquals(kind, error.kind) }
    }

    @Test fun callerCancellationWaitsForRealRetirementAndDiscardsLateSuccess() = runBlocking {
        withTimeout(3000) {
            val call = Call(); val jobs = JoinedDocumentJobs(Backend(call))
            val run = async { jobs.run(request, byteArrayOf(1)) }
            call.started.await(); run.cancel(); call.cancelled.await()
            assertFalse(run.isCompleted)
            call.complete.complete(Unit); yield()
            assertFalse(run.isCompleted)
            call.retirement.complete(Unit); run.join()
            assertTrue(run.isCancelled); assertEquals(1, call.retireCount.get())
            jobs.close()
        }
    }
    @Test fun cancelledChildPipeFailureCannotEscapeAsAnUnrelatedWorkerError() = runBlocking {
        withTimeout(3000) {
            val call = Call(); call.failureOnExit = DocumentFailureKind.Worker
            val jobs = JoinedDocumentJobs(Backend(call))
            val run = async { jobs.run(request, byteArrayOf(1)) }
            call.started.await(); run.cancel(); call.cancelled.await()
            call.complete.complete(Unit); call.retirement.complete(Unit); run.join()
            assertTrue(run.isCancelled); assertTrue(currentCoroutineContext().isActive)
            jobs.close()
        }
    }
    @Test fun closeSignalsBeforeWaitingOnRunLockAndNeverDestroysAnActiveCall() = runBlocking {
        withTimeout(3000) {
            val call = Call(); val backend = Backend(call); val jobs = JoinedDocumentJobs(backend)
            val run = async { try { jobs.run(request, byteArrayOf(1)); null } catch (failure: DocumentFailure) { failure.kind } }
            call.started.await()
            val close = async { jobs.close() }
            call.cancelled.await(); assertFalse(close.isCompleted); assertEquals(0, backend.closes)
            call.complete.complete(Unit); yield(); assertFalse(close.isCompleted)
            call.retirement.complete(Unit)
            assertEquals(DocumentFailureKind.Cancelled, run.await()); close.await()
            assertEquals(1, backend.closes); assertFalse(jobs.pcRedaction)
            jobs.close(); assertEquals(1, backend.closes)
            failure(DocumentFailureKind.Unavailable) { jobs.run(request, byteArrayOf(2)) }
        }
    }
    @Test fun uncertainRetirementRetainsTheSameOwnerAndBlocksNewWorkUntilCloseRetry() = runBlocking {
        val call = Call(); call.complete.complete(Unit); call.uncertain = true
        val backend = Backend(call); val jobs = JoinedDocumentJobs(backend)
        failure(DocumentFailureKind.Isolation) { jobs.run(request, byteArrayOf(1)) }
        failure(DocumentFailureKind.Isolation) { jobs.run(request, byteArrayOf(2)) }
        assertEquals(1, backend.calls); assertEquals(0, backend.closes)
        failure(DocumentFailureKind.Isolation) { jobs.close() }
        assertEquals(0, backend.closes)
        call.uncertain = false; call.retirement.complete(Unit)
        jobs.close(); assertEquals(1, backend.closes); assertEquals(3, call.retireCount.get())
    }
    @Test fun queuedRunCannotPassCloseAdmissionAfterReturningToTheSameSource() = runBlocking {
        withTimeout(3000) {
            val call = Call(); val backend = Backend(call); val jobs = JoinedDocumentJobs(backend)
            val first = async { try { jobs.run(request, byteArrayOf(1)); null } catch (error: DocumentFailure) { error.kind } }
            call.started.await()
            val second = async(start = CoroutineStart.UNDISPATCHED) { try { jobs.run(request, byteArrayOf(1)); null } catch (error: DocumentFailure) { error.kind } }
            val close = async(start = CoroutineStart.UNDISPATCHED) { jobs.close() }
            call.cancelled.await(); call.complete.complete(Unit); call.retirement.complete(Unit)
            first.await(); assertEquals(DocumentFailureKind.Unavailable, second.await()); close.await()
            assertEquals(1, backend.calls)
        }
    }
}
