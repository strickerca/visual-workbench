package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.Assert.*
import org.junit.Test
import kotlin.coroutines.CoroutineContext

class AgentCaptureObserverTest {
    @Test fun completeSnapshotsReplaceAndNeverPublishAuthority() = runBlocking {
        val f = Fixture()
        try {
            f.observer.attach(f.link)
            for (value in listOf(active(2uL, count = 3u), active(3uL, count = 1u),
                active(4uL, count = 0u, capturing = true), active(5uL, count = 0u), AgentCaptureStatus(6uL, 1uL))) {
                f.api.emit(value); settle { f.observer.state.value == value }
            }
            assertFalse(f.observer.state.value.known)
            assertEquals(0, f.api.publications)
        } finally { f.close() }
    }
    @Test fun lateReplyAcrossOfflineAndNewNativeEpochCannotRestorePriorGrantState() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.api.hook = { entered.complete(Unit); withContext(NonCancellable) { release.await() }; active(2uL) }
        try {
            f.observer.attach(f.link); withTimeout(3000) { entered.await() }
            f.observer.connectionChanged(f.link.status().copy(status = SyncStatus.Reconnecting))
            assertFalse(f.observer.state.value.known)
            f.api.current = AgentCaptureStatus(3uL, 3uL)
            f.observer.connectionChanged(f.link.status())
            release.complete(Unit)
            settle { f.observer.state.value.connectionEpoch == 3uL }
            assertFalse(f.observer.state.value.known)
            f.api.emit(active(4uL).copy(connectionEpoch = 3uL))
            settle { f.observer.state.value.known }
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun synchronousNativeEpochReadFencesReplyBeforeStatusCollectorRuns() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.api.hook = { entered.complete(Unit); release.await(); active(2uL) }
        try {
            f.observer.attach(f.link); withTimeout(3000) { entered.await() }
            f.api.current = AgentCaptureStatus(3uL, 2uL) // Native detached; app status flow still says Synced.
            release.complete(Unit); settle { f.api.waitCalls >= 2 }
            assertFalse(f.observer.state.value.known)
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun replacementJoinsOldProducerBeforeNewCapabilityAndNeverReusesItsReply() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.api.hook = { withContext(NonCancellable) { entered.complete(Unit); release.await() }; active(2uL) }
        try {
            f.observer.attach(f.link); withTimeout(3000) { entered.await() }
            val detached = async(start = CoroutineStart.UNDISPATCHED) { f.observer.detach() }
            assertFalse(detached.isCompleted); assertFalse(f.observer.state.value.known)
            release.complete(Unit); withTimeout(3000) { detached.await() }
            assertEquals(0, f.api.activeWaits)
            val replacement = FakeApi().also { it.current = active(10uL, count = 4u).copy(connectionEpoch = 7uL) }
            f.selected = replacement; f.observer.attach(FakeLink())
            settle { f.observer.state.value.activeGrantCount == 4u }
            f.api.emit(active(99uL, count = 0u)); yield()
            assertEquals(7uL, f.observer.state.value.connectionEpoch)
            assertEquals(4u, f.observer.state.value.activeGrantCount)
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun cancelledBeforeDispatchHasNoCapabilityCallsAndLeavesUnknown() = runBlocking {
        val dispatcher = QueueDispatcher(); val scope = CoroutineScope(SupervisorJob() + dispatcher)
        val api = FakeApi(); val observer = AgentCaptureObserver(scope) { api }
        observer.attach(FakeLink())
        val detached = async(start = CoroutineStart.UNDISPATCHED) { observer.detach() }
        dispatcher.drain(); withTimeout(3000) { detached.await() }
        assertEquals(0, api.statusCalls); assertEquals(0, api.waitCalls)
        assertFalse(observer.state.value.known); scope.cancel(); dispatcher.drain()
    }
    @Test fun unavailableCapabilityAndExpiryUseUnknownWordingInsteadOfInactive() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val observer = AgentCaptureObserver(scope) { throw SessionFailure(SessionFailureKind.Invalid) }
        try {
            observer.attach(FakeLink())
            assertFalse(observer.state.value.known)
            assertTrue(agentCaptureStatusText(observer.state.value).contains("unknown"))
            assertFalse(agentCaptureStatusText(observer.state.value).contains("no active"))
            assertTrue(agentCaptureStatusText(active(2uL, count = 0u, capturing = true)).contains("is active"))
        } finally { observer.detach(); scope.cancel() }
    }

    private suspend fun settle(check: () -> Boolean) = withTimeout(3000) { while (!check()) yield() }
    private class Fixture {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val api = FakeApi(); var selected = api; val link = FakeLink()
        val observer = AgentCaptureObserver(scope) { selected }
        suspend fun close() { observer.detach(); scope.cancel() }
    }
    private class FakeApi : WorkbenchAgentCaptureStatus {
        var current = AgentCaptureStatus(1uL, 1uL)
        private val updates = Channel<AgentCaptureStatus>(Channel.CONFLATED)
        var hook: (suspend () -> AgentCaptureStatus)? = null
        var waitCalls = 0; var statusCalls = 0; var activeWaits = 0; var publications = 0
        fun emit(value: AgentCaptureStatus) { current = value; updates.trySend(value) }
        override fun status(): AgentCaptureStatus { statusCalls++; return current }
        override suspend fun waitStatus(afterSequence: ULong): AgentCaptureStatus {
            waitCalls++; activeWaits++; val held = hook; hook = null
            try {
                if (held != null) return held()
                while (true) { val next = updates.receive(); if (next.sequence > afterSequence) return next }
            } finally { activeWaits-- }
        }
        override fun publish(value: LocalAgentCaptureSummary) { publications++; error("Display observer cannot publish authority") }
    }
    private class FakeLink : ProjectLink {
        override val endpoints = emptyList<SessionEndpoint>()
        override val changes = MutableStateFlow(SessionStatus(1uL, SyncStatus.Synced, SessionCarrier.QuicWifi,
            0u, 0u, 0uL, "synthetic", "synthetic-peer", null, null, 0u, null, null, null))
        override fun status() = changes.value
        override fun sendViewport(documentId: String, corners: List<Point>) = error("Indicator cannot move camera")
        override fun streamStroke(stroke: WorkbenchStroke, batch: SampleBatch, firstSample: UInt) = error("unused")
        override fun streamObject(preview: ObjectPreview) = error("unused")
        override fun streamNewObject(preview: NewObjectPreview) = error("unused")
        override suspend fun finishPreview(gestureId: String, cancel: Boolean) = error("unused")
        override suspend fun peerPreviews(documentId: String): PeerPreviews = error("unused")
        override suspend fun close() = Unit
    }
    private class QueueDispatcher : CoroutineDispatcher() {
        private val tasks = ArrayDeque<Runnable>()
        override fun dispatch(context: CoroutineContext, block: Runnable) { tasks.addLast(block) }
        fun drain() { var count = 0; while (tasks.isNotEmpty()) { check(++count < 1000); tasks.removeFirst().run() } }
    }
    private companion object {
        fun active(sequence: ULong, count: UInt = 2u, capturing: Boolean = false) =
            AgentCaptureStatus(sequence, 1uL, true, count, capturing, if (count == 0u) 0u else 30_000u)
    }
}
