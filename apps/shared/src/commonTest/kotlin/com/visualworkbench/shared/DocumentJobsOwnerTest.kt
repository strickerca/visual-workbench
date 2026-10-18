package com.visualworkbench.shared

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlin.test.*

class DocumentJobsOwnerTest {
    private class Jobs : DocumentJobs {
        override val pcRedaction: Boolean = true
        var closes = 0
        var fails = false
        var closing: CompletableDeferred<Unit>? = null
        var release: CompletableDeferred<Unit>? = null
        val reply = DocumentReply("a".repeat(64), "b".repeat(64),
            DocumentMetadata.Info(EditableKind.Svg, 1u, 1.0, 1.0, false, emptyList()), byteArrayOf())
        override suspend fun run(request: DocumentRequest, source: ByteArray): DocumentReply = reply
        override suspend fun close() {
            closes++
            closing?.complete(Unit)
            release?.await()
            if (fails) throw DocumentFailure(DocumentFailureKind.Isolation)
        }
    }

    @Test fun unavailableUntilInstalledAndCannotReplaceAnOwnedTransport(): Unit = runBlocking {
        val owner = DocumentJobsOwner()
        assertFalse(owner.held)
        assertFalse(owner.pcRedaction)
        assertEquals(DocumentFailureKind.Unavailable,
            assertFailsWith<DocumentFailure> { owner.run(DocumentRequest(DocumentOperation.SvgInfo), byteArrayOf(1)) }.kind)
        val jobs = Jobs()
        owner.install(jobs)
        owner.install(jobs)
        assertTrue(owner.pcRedaction)
        assertFailsWith<IllegalStateException> { owner.install(Jobs()) }
        assertSame(jobs.reply, owner.run(DocumentRequest(DocumentOperation.SvgInfo), byteArrayOf(1)))
        owner.close()
        assertFalse(owner.held)
        assertFalse(owner.pcRedaction)
        owner.close()
        assertEquals(1, jobs.closes)
        assertFailsWith<IllegalStateException> { owner.install(Jobs()) }
    }

    @Test fun failedRetirementKeepsTheActualOwnerAndCloseCanRetry(): Unit = runBlocking {
        val owner = DocumentJobsOwner()
        val jobs = Jobs().apply { fails = true }
        owner.install(jobs)
        assertEquals(DocumentFailureKind.Isolation, assertFailsWith<DocumentFailure> { owner.close() }.kind)
        assertTrue(owner.held)
        assertFalse(owner.pcRedaction)
        assertEquals(DocumentFailureKind.Unavailable,
            assertFailsWith<DocumentFailure> { owner.run(DocumentRequest(DocumentOperation.SvgInfo), byteArrayOf(1)) }.kind)
        assertFailsWith<IllegalStateException> { owner.install(Jobs()) }
        jobs.fails = false
        owner.close()
        assertFalse(owner.held)
        assertEquals(2, jobs.closes)
    }

    @Test fun cancellationOfCloseWaiterDoesNotReleaseAPendingTransport(): Unit = runBlocking {
        val entered = CompletableDeferred<Unit>()
        val release = CompletableDeferred<Unit>()
        val jobs = Jobs().apply { closing = entered; this.release = release }
        val owner = DocumentJobsOwner().apply { install(jobs) }
        val closing = async { owner.close() }
        entered.await()
        closing.cancel()
        assertFalse(closing.isCompleted)
        assertTrue(owner.held)
        assertEquals(DocumentFailureKind.Unavailable,
            assertFailsWith<DocumentFailure> { owner.run(DocumentRequest(DocumentOperation.SvgInfo), byteArrayOf(1)) }.kind)
        release.complete(Unit)
        closing.join()
        assertFalse(owner.held)
        assertEquals(1, jobs.closes)
    }

    @Test fun passwordsAndPrivateSourceAreExcludedFromDebugStrings() {
        val password = charArrayOf('p', 'w').concatToString()
        val request = DocumentRequest(DocumentOperation.PdfInfo, password)
        assertFalse(request.toString().contains(password))
        val info = info()
        val descriptor = EditableDescriptor("doc", "a".repeat(64), EditableKind.Pdf, emptyList(), info)
        assertEquals("EditableSource(bytes=3)", EditableSource(descriptor, byteArrayOf(65, 66, 67)).toString())
        val create = CreateDocument("private-path", "project", "doc", "device", "private-title", byteArrayOf(65),
            "a".repeat(64), EditableKind.Pdf, emptyList(), 0L, password)
        assertFalse(create.toString().contains(password))
        assertFalse(create.toString().contains("private-"))
        assertNull(DocumentFailure(DocumentFailureKind.Worker).cause)
    }

    @Test fun documentRevisionIncludesDeviceLamportAndDurableState() {
        val before = info()
        assertTrue(before.sameDocumentRevision(before.copy(title = "renamed locally")))
        assertFalse(before.sameDocumentRevision(before.copy(projectId = "other")))
        assertFalse(before.sameDocumentRevision(before.copy(hostSeq = 2uL)))
        assertFalse(before.sameDocumentRevision(before.copy(stateHash = "b".repeat(64))))
        assertFalse(before.sameDocumentRevision(before.copy(deviceId = "other")))
        assertFalse(before.sameDocumentRevision(before.copy(nextLamport = 2uL)))
    }

    private fun info() = ProjectInfo("project", "title", "device", 1uL, false, false, 1uL, "a".repeat(64), listOf("doc"))
}
