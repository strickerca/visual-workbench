package com.visualworkbench.shared

import com.visualworkbench.bindings.core.LiveSession
import com.visualworkbench.bindings.core.SessionException
import com.visualworkbench.bindings.core.FocusSignal as NFocusSignal
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.isActive
import kotlinx.coroutines.withContext
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

internal interface NativeFocusAccess { fun focusHandle(): LiveSession }
public actual fun markerFocus(link: ProjectLink): WorkbenchMarkerFocus = NativeMarkerFocus(
    link as? NativeFocusAccess ?: throw SessionFailure(SessionFailureKind.Invalid))
private fun NFocusSignal.common(): FocusSignal = FocusSignal(sequence, connectionEpoch, available)
private class NativeMarkerFocus(private val owner: NativeFocusAccess) : WorkbenchMarkerFocus {
    override val signals: Flow<FocusSignal> = flow {
        var sequence = 0uL
        while (currentCoroutineContext().isActive) {
            val next = withContext(Dispatchers.Default) {
                try { owner.focusHandle().waitFocus(sequence).common() }
                catch (error: SessionException) { throw sessionFailure(error) }
                catch (error: IllegalStateException) { throw SessionFailure(SessionFailureKind.Closed) }
            }
            if (next.sequence > sequence) { sequence = next.sequence; emit(next) }
        }
    }
    override fun signal(): FocusSignal = try { owner.focusHandle().focusSignal().common() }
    catch (error: SessionException) { throw sessionFailure(error) }
    catch (error: IllegalStateException) { throw SessionFailure(SessionFailureKind.Closed) }
    override suspend fun send(binding: WorkflowBinding, markerId: String?) {
        if (markerId != null && markerId.length != 36) throw WorkflowFailure(WorkflowFailureKind.Invalid)
        val native = workflowBinding(binding)
        settledWorkflow { cancellation -> owner.focusHandle().sendMarkerFocus(native, markerId, cancellation) }
    }
    override suspend fun peer(binding: WorkflowBinding): PeerMarkerFocus? {
        val native = workflowBinding(binding)
        return settledWorkflow { cancellation -> owner.focusHandle().peerMarkerFocus(native, cancellation)?.let {
            PeerMarkerFocus(workflowBinding(it.binding), it.markerId, it.instructionId, it.sequence, it.connectionEpoch)
        } }
    }
}
