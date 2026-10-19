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
import kotlinx.coroutines.withContext

/** The native URI admission already owns the reader. These are its real
 * prepare/build/publish owners, closed in reverse order after delivery. */
internal suspend fun publishAndroidGiantPicked(
    picked: GiantPickedOriginal,
    cache: GiantCacheBuildOptions,
    destination: (GiantOriginalInfo) -> GiantProjectOptions,
    progress: (String) -> Unit,
): WorkbenchProject {
    var prepared: GiantPreparedOriginal? = null
    var built: GiantBuiltImage? = null
    var project: WorkbenchProject? = null
    var delivered = false
    var failure: Throwable? = null
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
        currentCoroutineContext().ensureActive()
        delivered = true
        return checkNotNull(project)
    } catch (error: Throwable) {
        failure = error
        throw error
    } finally {
        withContext(NonCancellable) {
            var cleanup: Throwable? = null
            suspend fun settle(close: suspend () -> Unit) {
                try { close() } catch (error: Throwable) {
                    if (cleanup == null) cleanup = error else if (cleanup !== error) cleanup?.addSuppressed(error)
                }
            }
            if (!delivered) settle { project?.let { it.close() } }
            settle { built?.let { it.close() } }
            settle { prepared?.let { it.close() } }
            settle { picked.close() }
            if (cleanup != null) {
                if (failure != null) failure?.takeIf { it !== cleanup }?.addSuppressed(checkNotNull(cleanup))
                else if (delivered) runCatching { progress("The original was saved; temporary reader retirement needs a retry.") }
                else throw checkNotNull(cleanup)
            }
        }
    }
}
