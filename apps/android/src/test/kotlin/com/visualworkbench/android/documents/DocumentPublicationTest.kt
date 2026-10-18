package com.visualworkbench.android.documents

import com.visualworkbench.shared.*
import java.io.ByteArrayOutputStream
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class DocumentPublicationTest {
    private class Destination : DocumentDestination {
        val bytes = ByteArrayOutputStream()
        var opens = 0; var finishes = 0; var closes = 0; var removes = 0
        var failClose = false; var failOpen = false; var removeOk = true
        var afterWrite: suspend () -> Unit = {}
        var beforeClose: suspend () -> Unit = {}
        override suspend fun open() { opens++; if (failOpen) error("private-provider-path") }
        override suspend fun write(bytes: ByteArray, offset: Int, length: Int) { this.bytes.write(bytes, offset, length); afterWrite() }
        override suspend fun finish() { finishes++ }
        override suspend fun close() { closes++; beforeClose(); if (failClose) error("synthetic close") }
        override suspend fun remove(): Boolean { removes++; return removeOk }
    }
    private fun copy(bytes: ByteArray = ByteArray(70000) { 7 }) = DocumentExport(bytes, "b".repeat(64), DocumentCopy.Original, false, "a".repeat(64),
        ProjectInfo("project", "title", "device", 1uL, false, false, 1uL, "state", listOf("doc")), emptyList())
    private suspend fun failure(kind: DocumentFailureKind, block: suspend () -> Unit) {
        try { block(); fail("expected fixed failure") } catch (error: DocumentFailure) { assertEquals(kind, error.kind) }
    }
    @Test fun exactOwnedBytesFinishBeforeSuccessWithoutRemovingDestination() = runBlocking {
        val destination = Destination(); val copy = copy()
        DocumentPublisher { "b".repeat(64) }.publish(destination, copy) { true }
        assertArrayEquals(copy.bytes, destination.bytes.toByteArray()); assertEquals(1, destination.finishes); assertEquals(0, destination.removes)
    }
    @Test fun rawProviderFailureCannotEchoPrivatePathsOrCauses() = runBlocking {
        val destination = Destination(); destination.failOpen = true
        try { DocumentPublisher { "b".repeat(64) }.publish(destination, copy()) { true }; fail("expected fixed worker failure") }
        catch (error: DocumentFailure) { assertEquals(DocumentFailureKind.Worker, error.kind); assertEquals("Worker", error.message); assertNull(error.cause) }
        assertEquals(1, destination.removes)
    }
    @Test fun sourceOutputHashMismatchCleansTheUnusedNewDestination() = runBlocking {
        val destination = Destination()
        failure(DocumentFailureKind.Verification) { DocumentPublisher { "c".repeat(64) }.publish(destination, copy()) { true } }
        assertEquals(0, destination.opens); assertEquals(1, destination.removes)
    }
    @Test fun staleDuringWritingCannotBecomeReadyAndRemovesOwnedPartial() = runBlocking {
        var current = true; val destination = Destination(); destination.afterWrite = { current = false }
        failure(DocumentFailureKind.Stale) { DocumentPublisher { "b".repeat(64) }.publish(destination, copy()) { current } }
        assertEquals(0, destination.finishes); assertEquals(1, destination.removes); assertTrue(destination.bytes.size() < 70000)
    }
    @Test fun rollbackFailureExplicitlyReportsPossiblePartialFile() = runBlocking {
        val destination = Destination(); destination.removeOk = false
        try { DocumentPublisher { "b".repeat(64) }.publish(destination, copy()) { false }; fail("expected partial-file failure") }
        catch (error: DocumentPublicationFailure) { assertTrue(requireNotNull(error.message).contains("partial file may remain")) }
        assertEquals(0, destination.opens); assertEquals(1, destination.removes)
    }
    @Test fun failedCloseRetainsOwnerBlocksNewWriteAndRetriesThatSameOwner() = runBlocking {
        val publisher = DocumentPublisher { "b".repeat(64) }; val first = Destination(); first.failClose = true
        failure(DocumentFailureKind.Isolation) { publisher.publish(first, copy()) { true } }
        assertEquals(0, first.removes)
        val second = Destination()
        failure(DocumentFailureKind.Isolation) { publisher.publish(second, copy()) { true } }
        assertEquals(0, second.opens); assertEquals(1, second.removes)
        first.failClose = false; publisher.close(); assertEquals(1, first.removes)
    }
    @Test fun callerCancellationWaitsForActualWriterCloseBeforeRollback() = runBlocking {
        withTimeout(3000) {
            val wrote = CompletableDeferred<Unit>(); val writing = CompletableDeferred<Unit>(); val joined = CompletableDeferred<Unit>()
            val destination = Destination()
            destination.afterWrite = { wrote.complete(Unit); writing.await() }
            destination.beforeClose = { joined.await() }
            val operation = launch { DocumentPublisher { "b".repeat(64) }.publish(destination, copy()) { true } }
            wrote.await(); operation.cancel(); yield()
            assertFalse(operation.isCompleted); assertEquals(0, destination.removes)
            joined.complete(Unit); operation.join(); assertEquals(1, destination.removes)
        }
    }
}
