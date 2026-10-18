package com.visualworkbench.desktop.capture

import com.visualworkbench.shared.WorkbenchProject
import kotlinx.coroutines.CancellationException

/** Bound to EditorController.captureDestination and the actual NativeProject:
 * delivery errors are handled there; its finally joins close under
 * NonCancellable, mapping native cancellation to a non-coroutine exception.
 * An escaping non-cancellation failure therefore leaves close unproved. Pass
 * the concrete project unchanged and retain that failure, never close twice. */
internal suspend fun deliverCaptureProject(
    project: WorkbenchProject,
    destination: suspend (WorkbenchProject) -> Unit,
    retainFailure: (WorkbenchProject, Exception) -> Unit,
) {
    try { destination(project) }
    catch (error: CancellationException) { throw error }
    catch (error: Exception) { retainFailure(project, error); throw error }
}
