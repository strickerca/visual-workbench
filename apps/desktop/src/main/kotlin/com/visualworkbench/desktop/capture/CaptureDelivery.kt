package com.visualworkbench.desktop.capture

import com.visualworkbench.shared.WorkbenchProject
import com.visualworkbench.shared.ProjectInfo
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/** Bound to EditorController.captureDestination and the actual NativeProject:
 * delivery errors are handled there; its finally joins close under
 * NonCancellable, mapping native cancellation to a non-coroutine exception.
 * An escaping non-cancellation failure therefore leaves close unproved. Pass
 * the concrete project unchanged and retain that failure, never close twice. */
internal suspend fun deliverCaptureProject(
    project: WorkbenchProject,
    destination: suspend (WorkbenchProject) -> Unit,
    observation: (suspend (ProjectInfo) -> Unit)? = null,
    observationUnavailable: () -> Unit = {},
    retainFailure: (WorkbenchProject, Exception) -> Unit,
) {
    // The destination owns close at invocation. Snapshot the final semantic
    // revision first; the optional observer receives metadata, never a project
    // borrow after that close. An observation error cannot strand delivery.
    val info = if (observation == null) null else withContext(NonCancellable) {
        try { project.info() } catch (_: Exception) { null }
    }
    try { destination(project) }
    catch (error: CancellationException) { throw error }
    catch (error: Exception) { retainFailure(project, error); throw error }
    if (observation != null) withContext(NonCancellable) {
        val unavailable = if (info == null) true else try { observation(info); false }
            catch (_: Exception) { true }
        if (unavailable) observationUnavailable()
    }
}
