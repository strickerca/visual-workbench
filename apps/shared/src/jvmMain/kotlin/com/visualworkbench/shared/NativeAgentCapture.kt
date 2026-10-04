package com.visualworkbench.shared

import com.visualworkbench.bindings.core.AgentCaptureDisplay
import com.visualworkbench.bindings.core.SessionException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Reuses the existing borrowed LiveSession access, without acquiring another
 * native runtime or granting the unrelated marker-focus capability. */
public actual fun agentCaptureStatus(link: ProjectLink): WorkbenchAgentCaptureStatus = NativeAgentCapture(
    link as? NativeFocusAccess ?: throw SessionFailure(SessionFailureKind.Invalid))
private fun AgentCaptureDisplay.common() = AgentCaptureStatus(sequence, connectionEpoch, known,
    activeGrantCount, activeCapture, remainingMs)
private class NativeAgentCapture(private val owner: NativeFocusAccess) : WorkbenchAgentCaptureStatus {
    override fun status(): AgentCaptureStatus = mapped { owner.focusHandle().captureGrantStatus().common() }
    override suspend fun waitStatus(afterSequence: ULong): AgentCaptureStatus = withContext(Dispatchers.Default) {
        try { owner.focusHandle().waitCaptureGrantStatus(afterSequence).common() }
        catch (error: SessionException) { throw sessionFailure(error) }
        catch (_: IllegalStateException) { throw SessionFailure(SessionFailureKind.Closed) }
    }
    override fun publish(value: LocalAgentCaptureSummary) { mapped {
        if (value.known) owner.focusHandle().publishCaptureGrantStatus(value.activeGrantCount, value.activeCapture, value.remainingMs)
        else owner.focusHandle().invalidateCaptureGrantStatus()
    } }
    private fun <T> mapped(block: () -> T): T = try { block() }
    catch (error: SessionException) { throw sessionFailure(error) }
    catch (_: IllegalStateException) { throw SessionFailure(SessionFailureKind.Closed) }
}
