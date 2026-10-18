package com.visualworkbench.android.diagnostics

import com.visualworkbench.shared.DiagnosticEventCode
import com.visualworkbench.shared.DiagnosticMetrics
import com.visualworkbench.shared.DiagnosticPlatform
import com.visualworkbench.shared.PreparedDiagnosticsReport
import com.visualworkbench.shared.PrivateDiagnosticLog
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.OutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import kotlin.coroutines.CoroutineContext

class DiagnosticReportWriterTest {
    private fun report(): PreparedDiagnosticsReport = PrivateDiagnosticLog().also {
        it.append(0L, DiagnosticEventCode.AppStarted)
    }.prepare(DiagnosticPlatform.Android, DiagnosticMetrics(traceSampleCount = 3))

    @Test fun writesExactFrozenUtf8AndClosesOnce() {
        val log = PrivateDiagnosticLog().also { it.append(0L, DiagnosticEventCode.AppStarted) }
        val preview = log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics(traceSampleCount = 3))
        log.append(1L, DiagnosticEventCode.ExportSucceeded)
        val output = OwnedOutput()
        var opens = 0
        writePreparedDiagnosticsReport(preview) { opens += 1; output }
        assertArrayEquals(preview.jsonText.toByteArray(Charsets.UTF_8), output.bytes.toByteArray())
        assertEquals('\n'.code, output.bytes.toByteArray().last().toInt())
        assertEquals(1, opens)
        assertEquals(1, output.flushes)
        assertEquals(1, output.closes)
    }

    @Test fun writeFailureClosesOwnedStream() {
        val output = OwnedOutput(failWrite = true)
        assertThrows(IOException::class.java) { writePreparedDiagnosticsReport(report()) { output } }
        assertEquals(0, output.flushes)
        assertEquals(1, output.closes)
    }

    @Test fun flushFailureClosesOwnedStream() {
        val output = OwnedOutput(failFlush = true)
        assertThrows(IOException::class.java) { writePreparedDiagnosticsReport(report()) { output } }
        assertEquals(1, output.flushes)
        assertEquals(1, output.closes)
    }

    @Test fun destinationOpenFailureDoesNotCreateAWriter() {
        var opens = 0
        assertThrows(IOException::class.java) {
            writePreparedDiagnosticsReport(report()) { opens += 1; throw IOException("Unavailable") }
        }
        assertEquals(1, opens)
    }

    @Test fun closeFailureCannotBeReportedAsASuccessfulExport() {
        val output = OwnedOutput(failClose = true)
        assertThrows(IOException::class.java) { writePreparedDiagnosticsReport(report()) { output } }
        assertEquals(1, output.flushes)
        assertEquals(1, output.closes)
    }

    @Test fun closeFailurePreservesTheOriginalWriteFailure() {
        val output = OwnedOutput(failWrite = true, failClose = true)
        val failure = assertThrows(IOException::class.java) { writePreparedDiagnosticsReport(report()) { output } }
        assertEquals("Write failed", failure.message)
        assertEquals(1, failure.suppressed.size)
        assertEquals("Close failed", failure.suppressed.single().message)
        assertEquals(1, output.closes)
    }

    @Test fun cancellingAWaiterDoesNotCancelOrReleaseTheActualWriter() = runBlocking {
        val entered = CompletableDeferred<Unit>()
        val release = CountDownLatch(1)
        val output = OwnedOutput(flushEntered = entered, flushRelease = release)
        val preview = report()
        val writer = launch { withContext(Dispatchers.IO) { writePreparedDiagnosticsReport(preview) { output } } }
        try {
            withTimeout(5_000L) { entered.await() }
            val waiter = launch(start = CoroutineStart.UNDISPATCHED) { awaitPreparedReportWriter(writer) }
            waiter.cancelAndJoin()
            assertTrue(writer.isActive)
            assertEquals(0, output.closes)
            release.countDown()
            assertTrue(awaitPreparedReportWriter(writer))
            assertArrayEquals(preview.jsonText.toByteArray(Charsets.UTF_8), output.bytes.toByteArray())
            assertEquals(1, output.closes)
        } finally { release.countDown(); withTimeout(5_000L) { writer.join() } }
    }

    @Test fun expiredWaitRetainsOwnedStreamUntilLateSettlement() = runBlocking {
        val entered = CompletableDeferred<Unit>()
        val release = CountDownLatch(1)
        val output = OwnedOutput(flushEntered = entered, flushRelease = release)
        val preview = report()
        val writer = launch { withContext(Dispatchers.IO) { writePreparedDiagnosticsReport(preview) { output } } }
        try {
            withTimeout(5_000L) { entered.await() }
            assertFalse(awaitPreparedReportWriter(writer, waitMillis = 0L))
            assertTrue(writer.isActive)
            assertEquals(0, output.closes)
            release.countDown()
            assertTrue(awaitPreparedReportWriter(writer))
            assertEquals(1, output.closes)
        } finally { release.countDown(); withTimeout(5_000L) { writer.join() } }
    }

    @Test fun parentCancellationDoesNotClaimBlockedWriterIsRetired() = runBlocking {
        val entered = CompletableDeferred<Unit>()
        val release = CountDownLatch(1)
        val output = OwnedOutput(flushEntered = entered, flushRelease = release)
        val preview = report()
        val parent = Job()
        val writer = launch(parent) { withContext(Dispatchers.IO) { writePreparedDiagnosticsReport(preview) { output } } }
        try {
            withTimeout(5_000L) { entered.await() }
            parent.cancel()
            assertFalse(awaitPreparedReportWriter(writer, waitMillis = 0L))
            assertFalse(writer.isCompleted)
            assertEquals(0, output.closes)
            release.countDown()
            assertTrue(awaitPreparedReportWriter(writer))
            assertEquals(1, output.closes)
        } finally { release.countDown(); parent.cancel(); withTimeout(5_000L) { writer.join() } }
    }

    @Test fun cancellationWhileWriterIsBlockedRetainsOwnershipAndRecordsActualSuccess() = runBlocking {
        val entered = CompletableDeferred<Unit>()
        val release = CountDownLatch(1)
        val output = OwnedOutput(flushEntered = entered, flushRelease = release)
        val preview = report()
        val dispatcher = QueuedOwnerDispatcher()
        val parent = Job()
        val recorded = mutableListOf<DiagnosticEventCode>()
        val ownerThread = Thread.currentThread()
        val writer = CoroutineScope(parent + dispatcher).launchPreparedReportWriter(preview, { output }) {
            assertEquals(ownerThread, Thread.currentThread())
            recorded.add(it)
        }
        try {
            withTimeout(5_000L) { entered.await() }
            assertFalse(awaitPreparedReportWriter(writer, waitMillis = 0L))
            assertTrue(parent.children.any { it === writer })
            parent.cancel()
            assertFalse(writer.isCompleted)
            assertFalse(parent.isCompleted)
            assertTrue(parent.children.any { it === writer })
            assertTrue(recorded.isEmpty())
            assertEquals(0, output.closes)
            release.countDown()
            dispatcher.runUntilComplete(writer)
            assertArrayEquals(preview.jsonText.toByteArray(Charsets.UTF_8), output.bytes.toByteArray())
            assertEquals(1, output.closes)
            assertEquals(listOf(DiagnosticEventCode.ExportSucceeded), recorded)
            assertTrue(parent.isCompleted)
        } finally { release.countDown(); parent.cancel(); dispatcher.runUntilComplete(writer) }
    }

    @Test fun cancellationAfterStreamClosedBeforeOwnerResumesCannotReplaceSuccessWithFailure() {
        val output = OwnedOutput()
        val preview = report()
        val dispatcher = QueuedOwnerDispatcher()
        val parent = Job()
        val recorded = mutableListOf<DiagnosticEventCode>()
        val ownerThread = Thread.currentThread()
        val writer = CoroutineScope(parent + dispatcher).launchPreparedReportWriter(preview, { output }) {
            assertEquals(ownerThread, Thread.currentThread())
            recorded.add(it)
        }
        try {
            // Taking the queued owner continuation proves IO has already returned.
            // Cancel before running it, reproducing withContext's prompt-cancellation race.
            val resumeOwner = dispatcher.takeTask()
            assertArrayEquals(preview.jsonText.toByteArray(Charsets.UTF_8), output.bytes.toByteArray())
            assertEquals(1, output.closes)
            assertTrue(recorded.isEmpty())
            parent.cancel()
            assertFalse(writer.isCompleted)
            resumeOwner.run()
            dispatcher.runUntilComplete(writer)
            assertEquals(listOf(DiagnosticEventCode.ExportSucceeded), recorded)
        } finally { parent.cancel(); dispatcher.runUntilComplete(writer) }
    }

    @Test fun cancellationAfterSettlementDoesNotChangeOrRepeatTheRecordedOutcome() {
        val output = OwnedOutput()
        val dispatcher = QueuedOwnerDispatcher()
        val parent = Job()
        val recorded = mutableListOf<DiagnosticEventCode>()
        val writer = CoroutineScope(parent + dispatcher).launchPreparedReportWriter(report(), { output }, recorded::add)
        try {
            dispatcher.runUntilComplete(writer)
            assertEquals(listOf(DiagnosticEventCode.ExportSucceeded), recorded)
            parent.cancel()
            dispatcher.runUntilComplete(writer)
            assertEquals(listOf(DiagnosticEventCode.ExportSucceeded), recorded)
            assertEquals(1, output.closes)
        } finally { parent.cancel(); dispatcher.runUntilComplete(writer) }
    }

    @Test fun lateCloseFailureAfterCancellationStillRecordsTheActualFailure() = runBlocking {
        val entered = CompletableDeferred<Unit>()
        val release = CountDownLatch(1)
        val output = OwnedOutput(failClose = true, flushEntered = entered, flushRelease = release)
        val dispatcher = QueuedOwnerDispatcher()
        val parent = Job()
        val recorded = mutableListOf<DiagnosticEventCode>()
        val writer = CoroutineScope(parent + dispatcher).launchPreparedReportWriter(report(), { output }, recorded::add)
        try {
            withTimeout(5_000L) { entered.await() }
            parent.cancel()
            assertFalse(writer.isCompleted)
            assertFalse(parent.isCompleted)
            assertEquals(0, output.closes)
            assertTrue(recorded.isEmpty())
            release.countDown()
            dispatcher.runUntilComplete(writer)
            assertEquals(1, output.closes)
            assertEquals(listOf(DiagnosticEventCode.ExportFailed), recorded)
            assertTrue(parent.isCompleted)
        } finally { release.countDown(); parent.cancel(); dispatcher.runUntilComplete(writer) }
    }

    /** Deterministically holds the UI continuation after the real IO child returns. */
    private class QueuedOwnerDispatcher : CoroutineDispatcher() {
        private val tasks = LinkedBlockingQueue<Runnable>()
        override fun dispatch(context: CoroutineContext, block: Runnable) { tasks.add(block) }
        fun takeTask(): Runnable = checkNotNull(tasks.poll(5L, TimeUnit.SECONDS)) { "Owner continuation did not arrive" }
        fun runUntilComplete(writer: Job) {
            while (!writer.isCompleted) takeTask().run()
        }
    }

    private class OwnedOutput(
        private val failWrite: Boolean = false,
        private val failFlush: Boolean = false,
        private val failClose: Boolean = false,
        private val flushEntered: CompletableDeferred<Unit>? = null,
        private val flushRelease: CountDownLatch? = null,
    ) : OutputStream() {
        val bytes = ByteArrayOutputStream()
        var flushes = 0
        var closes = 0
        override fun write(value: Int) {
            if (failWrite) throw IOException("Write failed")
            bytes.write(value)
        }
        override fun write(data: ByteArray, offset: Int, length: Int) {
            if (failWrite) throw IOException("Write failed")
            bytes.write(data, offset, length)
        }
        override fun flush() {
            flushes += 1
            if (failFlush) throw IOException("Flush failed")
            flushEntered?.complete(Unit)
            flushRelease?.await()
        }
        override fun close() {
            closes += 1
            if (failClose) throw IOException("Close failed")
        }
    }
}
