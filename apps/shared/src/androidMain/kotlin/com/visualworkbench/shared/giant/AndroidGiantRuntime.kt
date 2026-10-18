package com.visualworkbench.shared.giant

import com.visualworkbench.shared.GiantImageFailure
import com.visualworkbench.shared.GiantImageFailureKind
import com.visualworkbench.shared.sessionResources
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
                assumeUntaggedSrgb = true, rootOperation = operation) // Native preparation already requires explicit consent.
            val build = AndroidJpegPyramid.build(input, sessionResources()) {
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
    } catch (_: CancellationException) { 2 }
    catch (error: GiantImageFailure) { when(error.kind) {
        GiantImageFailureKind.Cancelled -> 2
        GiantImageFailureKind.Backpressure -> 3
        GiantImageFailureKind.Unsupported -> 4
        GiantImageFailureKind.Invalid, GiantImageFailureKind.Stale -> 5
        GiantImageFailureKind.Memory -> 6
        GiantImageFailureKind.Limit -> 7
        GiantImageFailureKind.Closed -> 8
        else -> 1
    } }
    catch (error: TileBuildFailure) { if(error.reason == "busy") 3 else 5 }
    catch (_: OutOfMemoryError) { 7 }
    catch (_: Throwable) { 1 }
    finally {
        // Before acquisition, after joined build failure, or after successful
        // close there is no Java owner. Failed cleanup remains active/quarantined
        // and its later successful close supplies this same exact notification.
        if (!AndroidJpegPyramid.ownsRootOperation(operation)) ownerRetired(operation)
    }

    internal fun ownerRetired(operation: Long): Unit {
        if (operation > 0) {
            // A failed JNI notification conservatively leaves the native pin.
            // It must not convert successful Java retirement into a fake live
            // Java owner or release a different operation's source.
            try { retiredNative(operation) } catch (_: Throwable) { }
        }
    }

    /** Called only within build.withRootOutput by the nested native callback.
     * This describes its private typed owner; it never accepts external paths. */
    @JvmStatic private fun describeOutputForNative(output: AndroidTileBuild): Array<String> = arrayOf(
        output.directory.absolutePath, output.sourceSha256, output.sourceEncodedBytes.toString(),
        output.rawWidth.toString(), output.rawHeight.toString(), output.orientation.toString(),
        output.width.toString(), output.height.toString(), output.previewWidth.toString(), output.previewHeight.toString(),
    )
    @JvmStatic private external fun initializeNative(): Int
    @JvmStatic private external fun retiredNative(operation: Long): Int
    @JvmStatic private external fun cancelledNative(operation: Long): Boolean
    @JvmStatic private external fun consumeNative(operation: Long, output: AndroidTileBuild): Int
}
