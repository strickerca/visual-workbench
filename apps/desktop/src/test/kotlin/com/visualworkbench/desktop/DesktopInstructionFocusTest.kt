package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import org.junit.Assert.*
import org.junit.Test

class DesktopInstructionFocusTest {
    @Test fun actualLiveLinkJoinsFocusOwnerBeforeClosingItsBorrowedNativeLink() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        val link = FakeLink(); var armed = false; var settled = false
        val live = DesktopLiveLink(scope, { null }, {}, {}, {}, {
            if (armed) { entered.complete(Unit); release.await(); settled = true }
        })
        try {
            live.install(link) {}; armed = true
            val closing = async(start = CoroutineStart.UNDISPATCHED) { live.close() }
            withTimeout(3000) { entered.await() }
            assertFalse(closing.isCompleted); assertFalse(link.closed)
            release.complete(Unit); withTimeout(3000) { closing.await() }
            assertTrue(settled); assertTrue(link.closed); assertNull(live.attachment())
        } finally { release.complete(Unit); live.close(); scope.cancel() }
    }
    @Test fun focusCleanupFailureStillClosesTheOwnedLinkAndRetiresAttachment() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val link = FakeLink(); var armed = false
        val live = DesktopLiveLink(scope, { null }, {}, {}, {}, { if (armed) error("injected cleanup refusal") })
        try {
            live.install(link) {}; armed = true
            try { live.close(); fail("expected injected error") } catch (_: IllegalStateException) { }
            assertTrue(link.closed); assertNull(live.attachment())
        } finally { armed = false; live.close(); scope.cancel() }
    }
    private class FakeLink : ProjectLink {
        var closed = false
        override val endpoints = emptyList<SessionEndpoint>()
        override val changes = emptyFlow<SessionStatus>()
        override fun status() = SessionStatus(1uL, SyncStatus.Synced, null, 0u, 0u, 1uL, "state", "peer", null, null, 0u, null, null, null)
        override fun sendViewport(documentId: String, corners: List<Point>) = Unit
        override fun streamStroke(stroke: WorkbenchStroke, batch: SampleBatch, firstSample: UInt) = Unit
        override fun streamObject(preview: ObjectPreview) = Unit
        override fun streamNewObject(preview: NewObjectPreview) = Unit
        override suspend fun finishPreview(gestureId: String, cancel: Boolean) = Unit
        override suspend fun peerPreviews(documentId: String) = PeerPreviews(0uL, emptyList(), emptyList())
        override suspend fun close() { closed = true }
    }
}
