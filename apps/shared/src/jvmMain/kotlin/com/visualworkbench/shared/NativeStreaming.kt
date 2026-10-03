package com.visualworkbench.shared

import com.visualworkbench.bindings.core.Cancellation
import com.visualworkbench.bindings.core.CoreException
import com.visualworkbench.bindings.core.CreateFileImageProject
import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.createFileImageProject
import com.visualworkbench.bindings.core.ExportOptions as NExportOptions
import com.visualworkbench.bindings.core.FileExportOptions as NFileExportOptions
import com.visualworkbench.bindings.core.ImageFormat as NFormat
import com.visualworkbench.bindings.core.QueryRect
import com.visualworkbench.bindings.core.TransferException
import com.visualworkbench.bindings.core.AttachFileOptions as NAttachFileOptions
import com.visualworkbench.bindings.core.FileAssetKind as NFileAssetKind
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import java.util.concurrent.atomic.AtomicReference

private val streamScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
private val streamSlots = Semaphore(4)

/** Unlike a cancellable dispatcher return, this waits until the native producer
 * has released scratch handles before reporting cancellation to its caller. */
private suspend fun <T> settledStream(release: suspend (T) -> Unit = {}, block: suspend (Cancellation) -> T): T {
    if (!streamSlots.tryAcquire()) throw CoreFailure(CoreFailureKind.Backpressure)
    try {
        val token = Cancellation()
        try {
            val owned = AtomicReference<T?>(null)
            val task = streamScope.async {
                try { block(token).also { owned.set(it) } }
                catch (error: CoreException) { throw mapFailure(error) }
                catch (error: TransferException) { throw mapTransferFailure(error) }
            }
            try {
                val result = task.await()
                currentCoroutineContext().ensureActive()
                owned.set(null)
                return result
            } catch (error: CancellationException) {
                token.cancel()
                withContext(NonCancellable) {
                    try { task.await() } catch (_: Exception) { }
                    owned.getAndSet(null)?.let { release(it) }
                }
                throw error
            }
        } finally {
            // Cancellation joins the independent producer before destroying its
            // token. Construction failures still release the outer queue slot.
            token.destroy()
        }
    } finally {
        streamSlots.release()
    }
}

internal suspend fun <T> createFileNative(options: CreateFileProject, wrap: (ProjectSession) -> T): T {
    val handle = settledStream(release = { value: ProjectSession -> try { value.closeSession() } finally { value.destroy() } }) { cancel ->
        createFileImageProject(CreateFileImageProject(options.path, options.projectId, options.documentId, options.layerId,
            options.deviceId, options.title, options.sourcePath, options.workDirectory, options.nowMs,
            options.memoryBudgetBytes, options.maxEncodedBytes, options.maxScratchBytes), cancel)
    }
    // No dispatch/suspension boundary exists between owned native result and
    // facade wrapping. A wrapping exception releases the newly-created handle.
    try { return wrap(handle) }
    catch (error: Throwable) { withContext(NonCancellable) { try { handle.closeSession() } finally { handle.destroy() } }; throw error }
}

private fun transferOptions(options: FileExportOptions): NFileExportOptions {
    val export = options.export
    val format = when (export.format) {
        ImageFormat.Png8 -> NFormat.Png8
        ImageFormat.Png16 -> NFormat.Png16
        is ImageFormat.Jpeg -> NFormat.Jpeg(export.format.quality)
        ImageFormat.WebpLossless -> NFormat.WebpLossless
        is ImageFormat.WebpLossy -> NFormat.WebpLossy(export.format.quality)
    }
    val request = NExportOptions(export.documentId, format, export.marked,
        export.region?.let { QueryRect(it.x,it.y,it.width,it.height) },export.matteRgb,
        export.convertToSrgb,export.assumeUntaggedSrgb,export.allowDepthReduction,export.memoryBudgetBytes)
    return NFileExportOptions(request,options.outputPath,options.workDirectory,options.maxEncodedBytes,options.maxScratchBytes)
}
internal suspend fun exportFileNative(handle: ProjectSession, options: FileExportOptions): FileExportResult = settledStream { cancel ->
    transferResult(handle.exportImageFile(transferOptions(options),cancel))
}
internal suspend fun exportTransferNative(handle: ProjectSession, options: FileExportOptions): FileExportResult = settledStream { cancel ->
    transferResult(handle.exportTransferFile(transferOptions(options),cancel))
}
private fun transferResult(result: com.visualworkbench.bindings.core.FileExportResult): FileExportResult {
    val info = result.revision
    return FileExportResult(result.encodedBytes,result.blake3,result.metadataJson,
        ProjectInfo(info.projectId,info.title,info.deviceId,info.nextLamport,info.canUndo,info.canRedo,info.hostSeq,info.stateHash,info.documentIds),result.estimatedPeakBytes)
}
internal suspend fun preflightFileNative(handle: ProjectSession, options: FileExportOptions): FilePreflight = settledStream { cancel ->
    val result = handle.preflightImageFile(transferOptions(options), cancel)
    FilePreflight(result.width,result.height,result.sourceBitDepth,result.outputBitDepth,result.estimatedPeakBytes,
        result.buffered,result.requiresRenderValidation,transferInfo(result.revision))
}
internal suspend fun attachFileNative(handle: ProjectSession, options: AttachFileOptions): AttachedAsset = settledStream { cancel ->
    val result = handle.attachAssetFile(NAttachFileOptions(options.transactionId,options.deviceId,options.lamport,options.nowMs,
        options.sourcePath,options.workDirectory,when(options.kind){FileAssetKind.Image -> NFileAssetKind.IMAGE; FileAssetKind.Mp4 -> NFileAssetKind.MP4},
        options.memoryBudgetBytes,options.maxEncodedBytes,options.maxScratchBytes),cancel)
    AttachedAsset(result.assetId,result.byteSize,result.format,transferInfo(result.revision))
}
private fun transferInfo(info: com.visualworkbench.bindings.core.ProjectInfo): ProjectInfo = ProjectInfo(
    info.projectId,info.title,info.deviceId,info.nextLamport,info.canUndo,info.canRedo,info.hostSeq,info.stateHash,info.documentIds,
)
internal fun mapTransferFailure(error: TransferException): TransferFailure = when(error) {
    is TransferException.Dimensions -> TransferFailure(TransferFailureKind.Dimensions,width=error.width,height=error.height,dimensionLimit=error.limit,cause=error)
    is TransferException.PixelLimit -> TransferFailure(TransferFailureKind.PixelLimit,width=error.width,height=error.height,maxPixels=error.maxPixels,cause=error)
    is TransferException.Memory -> TransferFailure(TransferFailureKind.Memory,estimatedBytes=error.estimated,budgetBytes=error.budget,cause=error)
    is TransferException.Metadata -> TransferFailure(TransferFailureKind.Metadata,cause=error)
    is TransferException.Alpha -> TransferFailure(TransferFailureKind.Alpha,cause=error)
    is TransferException.Depth -> TransferFailure(TransferFailureKind.Depth,cause=error)
    is TransferException.EncodedLimit -> TransferFailure(TransferFailureKind.EncodedLimit,cause=error)
    is TransferException.ScratchLimit -> TransferFailure(TransferFailureKind.ScratchLimit,cause=error)
    is TransferException.Unsupported -> TransferFailure(TransferFailureKind.Unsupported,cause=error)
    is TransferException.Invalid -> TransferFailure(TransferFailureKind.Invalid,cause=error)
    is TransferException.Storage -> TransferFailure(TransferFailureKind.Storage,cause=error)
    is TransferException.Cancelled -> TransferFailure(TransferFailureKind.Cancelled,cause=error)
    is TransferException.Closed -> TransferFailure(TransferFailureKind.Closed,cause=error)
    is TransferException.Backpressure -> TransferFailure(TransferFailureKind.Backpressure,cause=error)
}
