package com.visualworkbench.android.editor

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class StreamRootIntegrationTest {
    private data class Handle(val id: String)

    @Test fun attachmentUsesExactProjectAndLinkIdentity() {
        val project = Handle("project"); val link = Handle("link")
        val attached = StreamRootAttachment(project, "document", 2u, 7, link, 9)
        assertTrue(attached.matches(project, "document", 2u, 7, link, 9))
        assertFalse(attached.matches(Handle("project"), "document", 2u, 7, link, 9))
        assertFalse(attached.matches(project, "document", 2u, 7, Handle("link"), 9))
        assertFalse(attached.matches(null, "document", 2u, 7, link, 9))
        assertFalse(attached.matches(project, "document", 2u, 7, null, 9))
    }

    @Test fun documentPageAndBothEpochsFenceTheAttachment() {
        val project = Any(); val link = Any()
        val attached = StreamRootAttachment(project, "document", null, 7, link, 9)
        assertFalse(attached.matches(project, "replacement", null, 7, link, 9))
        assertFalse(attached.matches(project, "document", 0u, 7, link, 9))
        assertFalse(attached.matches(project, "document", null, 8, link, 9))
        assertFalse(attached.matches(project, "document", null, 7, link, 10))
        // The attachment has no transport-status or host-revision predicate.
        // An unchanged target can inspect its retained durable Save after loss.
        assertTrue(attached.matches(project, "document", null, 7, link, 9))
    }

    @Test fun failedCleanupRetainsTheExactOwnerAndBlocksReplacement() = runBlocking {
        var attempts = 0; val first = Any(); val replacement = Any()
        val owners = StreamRootOwner<Any> { value ->
            assertSame(first, value)
            if (++attempts == 1) error("cleanup retained")
        }
        owners.retain(first)
        assertTrue(runCatching { owners.retire() }.isFailure)
        assertSame(first, owners.value)
        assertTrue(runCatching { owners.retain(replacement) }.isFailure)
        owners.retire()
        assertNull(owners.value); assertEquals(2, attempts)
        owners.retain(replacement); assertSame(replacement, owners.value)
    }

    @Test fun simultaneousRetirementJoinsOneActualClose() = runBlocking {
        val entered = CompletableDeferred<Unit>(); val released = CompletableDeferred<Unit>(); var closes = 0
        val retained = Any()
        val owners = StreamRootOwner<Any> { closes++; entered.complete(Unit); released.await() }
        owners.retain(retained)
        val first = launch(start = CoroutineStart.UNDISPATCHED) { owners.retire() }
        entered.await()
        val second = launch(start = CoroutineStart.UNDISPATCHED) { owners.retire() }
        assertSame(retained, owners.value); assertFalse(first.isCompleted); assertFalse(second.isCompleted)
        released.complete(Unit); joinAll(first, second)
        assertEquals(1, closes); assertNull(owners.value)
    }

    @Test fun cancellationStillJoinsTheRetainedRelease() = runBlocking {
        val entered = CompletableDeferred<Unit>(); val released = CompletableDeferred<Unit>(); val retained = Any()
        val owners = StreamRootOwner<Any> { entered.complete(Unit); released.await() }
        owners.retain(retained)
        val closing = launch(start = CoroutineStart.UNDISPATCHED) { owners.retire() }
        entered.await(); closing.cancel(); yield()
        assertFalse(closing.isCompleted); assertSame(retained, owners.value)
        released.complete(Unit); closing.join(); assertNull(owners.value)
    }

    @Test fun ownershipTransfersWithoutClosingTheBorrowedReceiver() = runBlocking {
        var receiverCloses = 0; var annotationCloses = 0
        val receiver = Any(); val annotation = Any()
        val receivers = StreamRootOwner<Any> { receiverCloses++ }
        val annotations = StreamRootOwner<Any> { annotationCloses++; receiverCloses++ }
        receivers.retain(receiver)
        annotations.retain(annotation) // New owner exists before the old slot releases it.
        receivers.transferred(receiver)
        receivers.retire(); assertEquals(0, receiverCloses)
        annotations.retire(); assertEquals(1, annotationCloses); assertEquals(1, receiverCloses)
        annotations.retire(); assertEquals(1, receiverCloses)
    }
}
