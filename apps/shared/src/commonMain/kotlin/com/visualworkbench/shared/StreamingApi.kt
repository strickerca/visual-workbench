package com.visualworkbench.shared

/** File paths refer to caller-owned app-private staging, never a shared URI.
 * Calls settle native work before propagating cancellation, so staging can be
 * removed in finally. Native publication is no-clobber and preserves originals. */
public interface WorkbenchStreamingCore {
    public suspend fun createFile(options: CreateFileProject): WorkbenchProject
}
public interface WorkbenchStreamingProject {
    public suspend fun exportFile(options: FileExportOptions): FileExportResult
}
/** Additive transfer capabilities; ordinary assets do not create a timeline. */
public interface WorkbenchFileTransfers {
    public suspend fun preflightFile(options: FileExportOptions): FilePreflight
    public suspend fun exportTransfer(options: FileExportOptions): FileExportResult
    public suspend fun attachFile(options: AttachFileOptions): AttachedAsset
}
public enum class TransferFailureKind {
    Dimensions, PixelLimit, Memory, Metadata, Alpha, Depth, EncodedLimit, ScratchLimit,
    Unsupported, Invalid, Storage, Cancelled, Closed, Backpressure,
}
public class TransferFailure(
    public val kind: TransferFailureKind,
    public val width: UInt? = null, public val height: UInt? = null,
    public val dimensionLimit: UInt? = null,
    public val maxPixels: ULong? = null,
    public val estimatedBytes: ULong? = null, public val budgetBytes: ULong? = null,
    cause: Throwable? = null,
) : Exception(kind.name, cause)
public data class FilePreflight(
    public val width: UInt, public val height: UInt,
    public val sourceBitDepth: UByte, public val outputBitDepth: UByte,
    public val estimatedPeakBytes: ULong, public val buffered: Boolean,
    /** Marked geometry and result assets are checked again before rendering. */
    public val requiresRenderValidation: Boolean, public val revision: ProjectInfo,
)
public enum class FileAssetKind { Image, Mp4 }
public data class AttachFileOptions(
    public val transactionId: String, public val deviceId: String,
    public val lamport: ULong, public val nowMs: Long,
    public val sourcePath: String, public val workDirectory: String, public val kind: FileAssetKind,
    public val memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL,
    public val maxEncodedBytes: ULong = 64uL * 1024uL * 1024uL,
    public val maxScratchBytes: ULong = 400_000_000uL,
)
public data class AttachedAsset(
    public val assetId: String, public val byteSize: ULong, public val format: String,
    public val revision: ProjectInfo,
)
public data class CreateFileProject(
    public val path: String, public val projectId: String, public val documentId: String,
    public val layerId: String, public val deviceId: String, public val title: String,
    public val sourcePath: String, public val workDirectory: String, public val nowMs: Long,
    public val memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL,
    public val maxEncodedBytes: ULong = 64uL * 1024uL * 1024uL,
    public val maxScratchBytes: ULong = 400_000_000uL,
)
public data class FileExportOptions(
    public val export: ExportOptions, public val outputPath: String, public val workDirectory: String,
    public val maxEncodedBytes: ULong = 512uL * 1024uL * 1024uL,
    public val maxScratchBytes: ULong = 400_000_000uL,
)
public data class FileExportResult(
    public val encodedBytes: ULong, public val blake3: String, public val metadataJson: String,
    public val revision: ProjectInfo,
    /** Compositor/encoder reservation, not process RSS or the earlier source
     * decode peak. Decode is checked separately against the same memory budget. */
    public val estimatedPeakBytes: ULong,
)
