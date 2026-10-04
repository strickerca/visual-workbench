package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.Assert.*
import org.junit.Test

class MarkerFocusFollowerTest {
    @Test fun followStartsDisabledAndNeverEchoesAReceivedSelection() = runBlocking {
        val f = Fixture()
        try {
            f.start(); assertFalse(f.follower.state.value.enabled); assertEquals(0, f.peer.calls)
            f.follower.enabled(true); f.settled()
            assertEquals(listOf(f.peer.reply), f.selected)
            assertEquals(0, f.peer.sends)
            f.follower.detach(); assertFalse(f.follower.state.value.enabled)
            f.follower.attach(f.peer); assertFalse(f.follower.state.value.enabled)
        } finally { f.close() }
    }
    @Test fun dirtyDraftBlocksSelectionUntilExplicitlyResolved() = runBlocking {
        val f = Fixture(); f.allow = false
        try {
            f.start(); f.follower.enabled(true); f.settled()
            assertTrue(f.selected.isEmpty()); assertNotNull(f.follower.state.value.message)
            f.allow = true; f.follower.revisionChanged(); f.settled()
            assertEquals(1, f.selected.size); assertNull(f.follower.state.value.message)
        } finally { f.close() }
    }
    @Test fun delayedQueryCannotCrossAnOpsRevisionPublication() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.peer.hook = { if (f.peer.calls == 1) { entered.complete(Unit); release.await() } }
        try {
            f.start(); f.follower.enabled(true); withTimeout(3000) { entered.await() }
            f.expected = f.expected.copy(stateHash = "new-visible-hash")
            f.peer.reply = f.peer.reply!!.copy(binding = f.expected, sequence = 2uL)
            f.follower.revisionChanged(); release.complete(Unit); f.settled()
            assertEquals(1, f.selected.size); assertEquals(f.expected, f.selected.single().binding)
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun delayedQueryCannotRepopulateSelectionAfterOfflineOrReconnect() = runBlocking {
        for (reconnect in listOf(false, true)) {
            val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
            f.peer.hook = { if (f.peer.calls == 1) { entered.complete(Unit); release.await() } }
            try {
                f.start(); f.follower.enabled(true); withTimeout(3000) { entered.await() }
                f.peer.emit(FocusSignal(2uL, 2uL, false))
                if (reconnect) {
                    f.peer.reply = f.peer.reply!!.copy(connectionEpoch = 3uL, sequence = 1uL)
                    f.peer.emit(FocusSignal(3uL, 3uL, true))
                }
                release.complete(Unit); f.settled()
                if (reconnect) { assertEquals(1, f.selected.size); assertEquals(3uL, f.selected.single().connectionEpoch) }
                else { assertTrue(f.selected.isEmpty()); assertNull(f.follower.state.value.latest) }
            } finally { release.complete(Unit); f.close() }
        }
    }
    @Test fun synchronousNativeSignalClosesTheFlowDeliveryWindow() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.peer.hook = { entered.complete(Unit); release.await() }
        try {
            f.start(); f.follower.enabled(true); withTimeout(3000) { entered.await() }
            f.peer.instant = FocusSignal(2uL, 2uL, false) // Native changed; flow has not resumed yet.
            release.complete(Unit); f.settled()
            assertTrue(f.selected.isEmpty())
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun disablingFollowDuringQueryPreventsLateFocus() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.peer.hook = { entered.complete(Unit); release.await() }
        try {
            f.start(); f.follower.enabled(true); withTimeout(3000) { entered.await() }
            f.follower.enabled(false); release.complete(Unit); f.settled()
            assertTrue(f.selected.isEmpty()); assertFalse(f.follower.state.value.enabled)
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun detachJoinsAnActualLateNativeProducerBeforeBorrowedLinkCanClose() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.peer.hook = { withContext(NonCancellable) { entered.complete(Unit); release.await() } }
        try {
            f.start(); f.follower.enabled(true); withTimeout(3000) { entered.await() }
            val detached = async(start = CoroutineStart.UNDISPATCHED) { f.follower.detach() }
            assertFalse(detached.isCompleted); assertFalse(f.follower.state.value.enabled)
            release.complete(Unit); withTimeout(3000) { detached.await() }
            assertTrue(f.selected.isEmpty()); assertEquals(0, f.peer.active)
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun focusBeforeOpsCanBeResolvedByTheNewRevisionWithoutAnotherPeerMessage() = runBlocking {
        val f = Fixture(); val accepted = f.expected.copy(hostSeq = 9uL, stateHash = "accepted")
        f.peer.reply = f.peer.reply!!.copy(binding = accepted)
        try {
            f.start(); f.follower.enabled(true); f.settled(); assertTrue(f.selected.isEmpty())
            f.expected = accepted; f.follower.revisionChanged(); f.settled()
            assertEquals(accepted, f.selected.single().binding)
            f.peer.reply = null; f.peer.emit(FocusSignal(2uL, 1uL, true)); f.settled()
            assertNull(f.follower.state.value.latest); assertEquals(1, f.selected.size)
        } finally { f.close() }
    }
    @Test fun newerDetachInvalidatesPendingAttachAndJoinsItsRetiringProducer() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.peer.hook = { withContext(NonCancellable) { entered.complete(Unit); release.await() } }
        try {
            f.start(); f.follower.enabled(true); withTimeout(3000) { entered.await() }
            val replacement = Fake(f.peer.reply!!.copy(markerId = "replacement"))
            val attach = async(start = CoroutineStart.UNDISPATCHED) { f.follower.attach(replacement) }
            val detach = async(start = CoroutineStart.UNDISPATCHED) { f.follower.detach() }
            assertFalse(attach.isCompleted); assertFalse(detach.isCompleted)
            release.complete(Unit); withTimeout(3000) { attach.await(); detach.await() }
            assertEquals(0, f.peer.active); assertEquals(0, replacement.calls)
            assertFalse(f.follower.state.value.available); assertFalse(f.follower.state.value.enabled)
            replacement.emit(FocusSignal(2uL, 1uL, true)); yield()
            assertFalse(f.follower.state.value.available); assertTrue(f.selected.isEmpty())
        } finally { release.complete(Unit); f.close() }
    }
    @Test fun newerAttachWinsWhileEarlierAttachIsJoiningAnOldNativeProducer() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        f.peer.hook = { withContext(NonCancellable) { entered.complete(Unit); release.await() } }
        try {
            f.start(); f.follower.enabled(true); withTimeout(3000) { entered.await() }
            val a = Fake(f.peer.reply!!.copy(markerId = "superseded"))
            val b = Fake(f.peer.reply!!.copy(markerId = "newest"))
            val first = async(start = CoroutineStart.UNDISPATCHED) { f.follower.attach(a) }
            val second = async(start = CoroutineStart.UNDISPATCHED) { f.follower.attach(b) }
            assertFalse(first.isCompleted); assertFalse(second.isCompleted)
            release.complete(Unit); withTimeout(3000) { first.await(); second.await() }
            f.follower.enabled(true); f.settled()
            assertEquals(0, a.calls); assertEquals("newest", f.selected.single().markerId)
            a.emit(FocusSignal(9uL, 2uL, false)); yield()
            assertTrue(f.follower.state.value.available)
        } finally { release.complete(Unit); f.close() }
    }
    private class Fixture {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        var expected = WorkflowBinding("project", "document", 7uL, "visible")
        var allow = true
        val selected = mutableListOf<PeerMarkerFocus>()
        val peer = Fake(PeerMarkerFocus(expected, "marker", "instruction", 1uL, 1uL))
        val follower = MarkerFocusFollower(scope, { expected }, { allow }, { selected.add(it) })
        suspend fun start() { follower.attach(peer) }
        suspend fun settled() { withTimeout(3000) { while (peer.active != 0) yield() }; yield() }
        suspend fun close() { follower.detach(); scope.cancel() }
    }
    private class Fake(var reply: PeerMarkerFocus?) : WorkbenchMarkerFocus {
        override val signals = MutableStateFlow(FocusSignal(1uL, 1uL, true))
        var instant = signals.value
        var calls = 0; var sends = 0; var active = 0
        var hook: suspend () -> Unit = {}
        fun emit(value: FocusSignal) { instant = value; signals.value = value }
        override fun signal() = instant
        override suspend fun send(binding: WorkflowBinding, markerId: String?) { sends++ }
        override suspend fun peer(binding: WorkflowBinding): PeerMarkerFocus? {
            calls++; active++; val captured = reply
            try { hook(); return captured?.takeIf { it.binding == binding } }
            finally { active-- }
        }
    }
}
