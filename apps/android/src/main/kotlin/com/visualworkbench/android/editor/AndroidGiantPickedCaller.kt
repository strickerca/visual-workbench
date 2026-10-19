package com.visualworkbench.android.editor

import com.visualworkbench.shared.GiantBuiltImage
import com.visualworkbench.shared.GiantCacheBuildOptions
import com.visualworkbench.shared.GiantOriginalInfo
import com.visualworkbench.shared.GiantPickedOriginal
import com.visualworkbench.shared.GiantPreparedOriginal
import com.visualworkbench.shared.GiantProjectOptions
import com.visualworkbench.shared.WorkbenchProject
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.isActive
import kotlinx.coroutines.withContext

/** The native URI admission already owns the reader. These are its real
 * prepare/build/publish owners, closed in reverse order after delivery. */
internal suspend fun publishAndroidGiantPicked(
    picked: GiantPickedOriginal,
    cache: GiantCacheBuildOptions,
    destination: (GiantOriginalInfo) -> GiantProjectOptions,
    progress: (String) -> Unit,
): WorkbenchProject {
    val caller = currentCoroutineContext()
    var prepared: GiantPreparedOriginal? = null
    var built: GiantBuiltImage? = null
    var project: WorkbenchProject? = null
    var failure: Throwable? = null
    var cleanup: Throwable? = null
    var projectClosed = false
    suspend fun settle(close: suspend () -> Unit) {
        try { close() } catch (error: Throwable) {
            if (cleanup == null) cleanup = error else if (cleanup !== error) cleanup?.addSuppressed(error)
        }
    }
    try {
        progress("Inspecting the complete original…")
        prepared = picked.prepare()
        val info = prepared.describe()
        // Android's admitted region producer is JPEG only. A PNG/RAW/other
        // original is not silently converted or passed to that producer.
        require(info.format.equals("jpeg", ignoreCase = true)) { "Large-image tiled import supports JPEG originals on this device." }
        currentCoroutineContext().ensureActive()
        progress("Building bounded original-resolution tiles…")
        built = prepared.build(cache)
        currentCoroutineContext().ensureActive()
        progress("Saving the verified original and tiles…")
        project = built.publish(destination(info))
        caller.ensureActive()
    } catch (error: Throwable) {
        failure = error
    } finally {
        withContext(NonCancellable) {
            settle { built?.let { it.close() } }
            settle { prepared?.let { it.close() } }
            settle { picked.close() }
            if (failure != null || !caller.isActive) {
                settle { project?.let { it.close() } }
                projectClosed = true
            }
        }
    }
    failure?.let { error -> cleanup?.takeIf { it !== error }?.let(error::addSuppressed); throw error }
    if (cleanup != null) runCatching { progress("The original was saved; temporary reader retirement needs a retry.") }
    try { caller.ensureActive() } catch (cancel: Throwable) {
        // Cancellation may arrive while owners settle or just after that
        // settlement. Never return an undelivered project in either window.
        if (!projectClosed) withContext(NonCancellable) { settle { project?.let { it.close() } } }
        cleanup?.takeIf { it !== cancel }?.let(cancel::addSuppressed)
        throw cancel
    }
    return checkNotNull(project)
}
