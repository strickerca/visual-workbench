package com.visualworkbench.desktop.remote

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import org.junit.Assert.*
import org.junit.Test

class RemoteWindowSelectionTest {
    private val first = RemoteWindowCandidate(11uL, 21u, 31uL, "Owned Paint", "mspaint.exe")
    private val replacement = RemoteWindowCandidate(12uL, 22u, 32uL, "Other owned Paint", "mspaint.exe")

    @Test fun grantRetryKeepsTheExistingNativeVideoSelection() = runBlocking {
        val remote = SelectionRemote(); val selection = RemoteWindowSelection(remote)
        selection.select(first)
        val refused = SessionFailure(SessionFailureKind.Invalid)
        remote.grantFailure = refused
        try { selection.grant(); fail("Grant refusal was swallowed") }
        catch (caught: SessionFailure) { assertSame(refused, caught) }
        assertEquals(first, selection.selected.value)
        remote.grantFailure = null
        selection.grant(); selection.grant()
        assertEquals(listOf(first), remote.selections)
        assertEquals(3, remote.grants)
    }

    @Test fun pendingOrFailedReplacementNeverGrantsThePreviousSelection() = runBlocking {
        val remote = SelectionRemote(); val selection = RemoteWindowSelection(remote)
        selection.select(first)
        val gate = CompletableDeferred<Unit>(); remote.selectGate = gate
        val refused = SessionFailure(SessionFailureKind.Authentication); remote.selectFailure = refused
        var failure: Exception? = null
        val changing = launch(start = CoroutineStart.UNDISPATCHED) {
            try { selection.select(replacement) } catch (caught: Exception) { failure = caught }
        }
        try {
            assertNull(selection.selected.value)
            assertGrantBlocked(selection)
            assertEquals(0, remote.grants)
        } finally { gate.complete(Unit); changing.join() }
        assertSame(refused, failure)
        assertNull(selection.selected.value)
        assertGrantBlocked(selection)
        assertEquals(listOf(first, replacement), remote.selections)
        assertEquals(0, remote.grants)
    }

    @Test fun grantWithoutSuccessfulSelectionNeverCallsNativeGrant() = runBlocking {
        val remote = SelectionRemote(); val selection = RemoteWindowSelection(remote)
        assertGrantBlocked(selection)
        assertNull(selection.selected.value)
        assertTrue(remote.selections.isEmpty()); assertEquals(0, remote.grants)
    }

    private suspend fun assertGrantBlocked(selection: RemoteWindowSelection) {
        try { selection.grant(); fail("Grant was admitted without a selected window") }
        catch (_: IllegalStateException) { }
    }
}

private class SelectionRemote : WorkbenchRemoteEdit {
    val selections = mutableListOf<RemoteWindowCandidate>()
    var grants = 0
    var selectGate: CompletableDeferred<Unit>? = null
    var selectFailure: Exception? = null
    var grantFailure: Exception? = null
    override val target = MutableStateFlow<RemoteTargetState?>(null)
    override val state = MutableStateFlow(RemoteEditState(1uL, RemoteEditStatus.Selecting, null, null, null, null, null))
    override val frames: Flow<RemoteVideoFrame> = emptyFlow()
    override suspend fun select(candidate: RemoteWindowCandidate) {
        selections += candidate
        selectGate?.await(); selectFailure?.let { throw it }
    }
    override suspend fun grant() { grants++; grantFailure?.let { throw it } }
    override fun registerConsumer(owner: RemoteConsumerOwner) = error("Unused")
    override fun invalidateConsumers() = error("Unused")
    override fun seal(reason: String) = error("Unused")
    override suspend fun windows(): List<RemoteWindowCandidate> = error("Unused")
    override suspend fun requestControl() = error("Unused")
    override suspend fun pause(reason: String) = error("Unused")
    override suspend fun requestKeyframe() = error("Unused")
    override fun reservePen(binding: RemoteTargetBinding, samples: UInt): Boolean = error("Unused")
    override suspend fun pen(binding: RemoteTargetBinding, sample: RemotePenSample): RemoteInputAdmission = error("Unused")
    override suspend fun discardFrame(ticket: ULong) = error("Unused")
    override suspend fun rendered(ticket: ULong, ptsUs: Long): RemoteRenderedAcknowledgment? = error("Unused")
    override suspend fun dispatch(binding: RemoteTargetBinding, action: RemoteEditorAction): RemoteCommandReceipt = error("Unused")
    override suspend fun close() = error("Unused")
}
