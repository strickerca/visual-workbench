package com.visualworkbench.shared

/** Optional local-file capability. Content URIs require a separately admitted
 * bounded staging owner before they can become a local source path. */
public interface WorkbenchGiantImportCore {
    public suspend fun openGiantOriginal(options: GiantOriginalOptions): GiantPickedOriginal
}
public class GiantOriginalOptions(public val sourcePath: String, public val workDirectory: String,
    public val maxOriginalBytes: ULong, public val maxWorkDiskBytes: ULong,
    public val assumeUntaggedSrgb: Boolean) {
    public override fun toString(): String = "GiantOriginalOptions"
}
public class GiantOriginalInfo(public val assetId: String, public val sha256: String,
    public val encodedBytes: ULong, public val rawWidth: UInt, public val rawHeight: UInt,
    public val width: UInt, public val height: UInt, public val orientation: UByte,
    public val format: String) {
    public override fun toString(): String = "GiantOriginalInfo(width=$width, height=$height, format=$format)"
}
public data class GiantCacheBuildOptions(public val diskCacheBytes: ULong,
    public val timeoutMs: ULong, public val assumeUntaggedSrgb: Boolean)
public class GiantProjectOptions(public val path: String, public val projectId: String,
    public val documentId: String, public val layerId: String, public val deviceId: String,
    public val title: String, public val nowMs: Long, public val diskCacheBytes: ULong) {
    public override fun toString(): String = "GiantProjectOptions"
}
/** close settles actual cleanup. Retain a failed owner; quota is never credited
 * merely because a coroutine, wrapper or parent wishes to stop. */
public interface GiantPickedOriginal {
    public suspend fun prepare(): GiantPreparedOriginal
    public suspend fun close(): Unit
}
public interface GiantPreparedOriginal {
    public suspend fun describe(): GiantOriginalInfo
    public suspend fun build(options: GiantCacheBuildOptions): GiantBuiltImage
    public suspend fun close(): Unit
}
public interface GiantBuiltImage {
    /** An accepted destination survives cancellation or failed result delivery. */
    public suspend fun publish(options: GiantProjectOptions): WorkbenchProject
    public suspend fun rebuild(project: WorkbenchProject, binding: WorkflowBinding): Unit
    public suspend fun close(): Unit
}
