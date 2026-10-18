package com.visualworkbench.shared.giant

import com.visualworkbench.shared.sharedSessionResources
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.runBlocking
import java.io.File

/** Fixed root bridge; no installable runner callback, Activity or caller output
 * directory. Native pins this exact class and invokes its private method only
 * from an admitted PreparedOriginal worker. Output adoption is a nested call on
 * that same worker, followed by joined Android retirement before return. */
internal object AndroidGiantRuntime {
    private var installed = false
    @Synchronized fun install(): Unit {
        if (!installed) installed = initializeNative() == 0
    }

    @JvmStatic private fun buildPrepared(
        operation: Long, source: String, work: String, sha256: String,
        rawWidth: Int, rawHeight: Int, orientation: Int, encodedBytes: Long,
        maxDiskBytes: Long,
    ): Int = try {
        runBlocking {
            if (cancelledNative(operation)) throw CancellationException("giant build cancelled")
            val input = JpegTileInput.fromRootPrepared(File(source), File(work), sha256,
                rawWidth, rawHeight, orientation, encodedBytes, maxDiskBytes,
                assumeUntaggedSrgb = true) // Native preparation already requires explicit consent.
            val build = AndroidJpegPyramid.build(input, sharedSessionResources()) {
                if (cancelledNative(operation)) throw CancellationException("giant build cancelled")
            }
            try {
                build.withRootOutput { live ->
                    if (cancelledNative(operation)) throw CancellationException("giant build cancelled")
                    if (consumeNative(operation, live) != 0) throw TileBuildFailure("native output adoption")
                }
            } finally {
                // Even success cannot escape this exact owner before cleanup and
                // its 80 MiB reservation/slot have actually retired. Failure is
                // quarantined by ProducerRuntime and prevents native publication.
                build.close()
            }
        }
        0
    } catch (_: Throwable) { 1 }

    /** Called only within build.withRootOutput by the nested native callback.
     * This describes its private typed owner; it never accepts external paths. */
    @JvmStatic private fun describeOutputForNative(output: AndroidTileBuild): Array<String> = arrayOf(
        output.directory.absolutePath, output.sourceSha256, output.sourceEncodedBytes.toString(),
        output.rawWidth.toString(), output.rawHeight.toString(), output.orientation.toString(),
        output.width.toString(), output.height.toString(), output.previewWidth.toString(), output.previewHeight.toString(),
    )
    @JvmStatic private external fun initializeNative(): Int
    @JvmStatic private external fun cancelledNative(operation: Long): Boolean
    @JvmStatic private external fun consumeNative(operation: Long, output: AndroidTileBuild): Int
}
