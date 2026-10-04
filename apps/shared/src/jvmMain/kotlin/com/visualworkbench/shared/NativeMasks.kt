package com.visualworkbench.shared

import com.visualworkbench.bindings.core.Cancellation
import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.SelectionException
import com.visualworkbench.bindings.core.SelectionBinding as NBinding
import com.visualworkbench.bindings.core.SelectionVersion as NVersion
import com.visualworkbench.bindings.core.SelectionRegion as NRegion
import com.visualworkbench.bindings.core.SelectionSnapshot as NSnapshot
import com.visualworkbench.bindings.core.SelectionEdit as NEdit
import com.visualworkbench.bindings.core.SelectionTarget as NTarget
import com.visualworkbench.bindings.core.SelectionOperation as NOperation
import com.visualworkbench.bindings.core.SelectionCombine as NCombine
import com.visualworkbench.bindings.core.SelectionExportOptions as NExport
import com.visualworkbench.bindings.core.SelectionExportKind as NExportKind
import com.visualworkbench.bindings.core.ProjectInfo as NInfo
import com.visualworkbench.bindings.core.Point as NPoint
import com.visualworkbench.bindings.core.QueryRect
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore

private val selectionScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
private val selectionSlots = Semaphore(4)

internal interface SelectionCancellation {
    fun cancel()
    fun release()
}
private class NativeSelectionCancellation : SelectionCancellation {
    val token = Cancellation()
    override fun cancel() { token.cancel() }
    override fun release() { token.destroy() }
}

/** Shared by every selection entry point. The independent bounded producer
 * settles before a cancelled caller may remove its private export directory. */
internal suspend fun <C : SelectionCancellation, T> settledSelection(
    factory: () -> C, scope: CoroutineScope = selectionScope, slots: Semaphore = selectionSlots,
    block: suspend (C) -> T,
): T {
    currentCoroutineContext().ensureActive()
    if (!slots.tryAcquire()) throw SelectionFailure(SelectionFailureKind.Backpressure)
    try {
        val token = factory()
        try {
            val producer = scope.async { block(token) }
            try {
                val result = producer.await()
                currentCoroutineContext().ensureActive()
                return result
            } catch (cancelled: CancellationException) {
                try { token.cancel() } catch (signalFailure: Exception) { cancelled.addSuppressed(signalFailure) }
                withContext(NonCancellable) {
                    try { producer.await() } catch (_: Exception) { }
                }
                throw cancelled
            }
        } finally { token.release() }
    } finally { slots.release() }
}

public actual fun WorkbenchProject.selections(): WorkbenchSelections {
    val handle = try { nativeProjectHandle(this) }
        catch (error: SessionFailure) { throw SelectionFailure(SelectionFailureKind.Invalid, cause = error) }
    return NativeSelections(handle)
}

private class NativeSelections(private val handle: ProjectSession) : WorkbenchSelections {
    private suspend fun <T> call(block: suspend (Cancellation) -> T): T =
        settledSelection(::NativeSelectionCancellation) { cancellation ->
            try { block(cancellation.token) }
            catch (error: SelectionException) { throw mapSelectionFailure(error) }
            catch (error: CancellationException) { throw error }
            // A retained capability may be used after its owning project's
            // generated object was destroyed. UniFFI refuses that pointer before
            // dispatch; report the capability lifetime instead of leaking a JVM error.
            catch (error: IllegalStateException) { throw SelectionFailure(SelectionFailureKind.Closed, cause = error) }
        }

    override suspend fun document(documentId: String): SelectionDocument = call { token ->
        val value = handle.selectionDocument(documentId, token)
        SelectionDocument(value.binding.common(), value.width, value.height, value.sourceBitDepth, value.revision.common())
    }
    override suspend fun snapshot(binding: SelectionBinding, objectId: String, memoryBudgetBytes: ULong): SelectionSnapshot = call {
        handle.selectionSnapshot(binding.native(), objectId, memoryBudgetBytes, it).common()
    }
    override suspend fun tiles(binding: SelectionBinding, selection: SelectionVersion,
        regions: List<SelectionRegion>, memoryBudgetBytes: ULong): SelectionTiles {
        if (regions.size !in 1..64) throw SelectionFailure(SelectionFailureKind.Limit)
        val request = regions.map { it.native() }
        return call { token ->
            val value = handle.selectionTiles(binding.native(), selection.native(), request, memoryBudgetBytes, token)
            SelectionTiles(value.binding.common(), value.selection.common(), value.tiles.map { SelectionTile(it.region.common(), it.coverage) })
        }
    }
    override suspend fun edit(options: SelectionEdit): SelectionReceipt {
        // Copy mutable caller lists before the first dispatcher/queue suspension.
        val request = options.ownedSelectionRequest().native()
        return call { token ->
            val value = handle.applySelection(request, token)
            SelectionReceipt(value.transactionId, value.snapshot.common(), value.revision.common())
        }
    }
    override suspend fun exportFile(options: SelectionExportOptions): SelectionExportReceipt {
        val request = NExport(options.binding.native(), options.selection.native(), options.kind.native(),
            options.region?.native(), options.workDirectory, options.outputPath, options.memoryBudgetBytes, options.maxEncodedBytes)
        return call { token ->
            val value = handle.exportSelectionFile(request, token)
            SelectionExportReceipt(value.binding.common(), value.selection.common(), value.kind.common(), value.region.common(),
                value.outputBitDepth, value.encodedBytes, value.blake3, value.metadataJson, value.revision.common(), value.estimatedPeakBytes)
        }
    }
}

private fun SelectionBinding.native(): NBinding = NBinding(projectId, documentId, sourceAssetId, hostSeq, stateHash)
private fun NBinding.common(): SelectionBinding = SelectionBinding(projectId, documentId, sourceAssetId, hostSeq, stateHash)
private fun SelectionVersion.native(): NVersion = NVersion(objectId, assetId, version)
private fun NVersion.common(): SelectionVersion = SelectionVersion(objectId, assetId, version)
private fun SelectionRegion.native(): NRegion = NRegion(x, y, width, height)
private fun NRegion.common(): SelectionRegion = SelectionRegion(x, y, width, height)
private fun NInfo.common(): ProjectInfo = ProjectInfo(projectId, title, deviceId, nextLamport, canUndo, canRedo, hostSeq, stateHash, documentIds)
private fun NSnapshot.common(): SelectionSnapshot = SelectionSnapshot(binding.common(), selection.common(), layerId,
    width, height, nonzeroBounds?.common(), editable, visible)
private fun SelectionCombine.native(): NCombine = when (this) {
    SelectionCombine.Add -> NCombine.ADD
    SelectionCombine.Subtract -> NCombine.SUBTRACT
    SelectionCombine.Intersect -> NCombine.INTERSECT
}
private fun SelectionExportKind.native(): NExportKind = when (this) {
    SelectionExportKind.Mask -> NExportKind.MASK
    SelectionExportKind.Cutout -> NExportKind.CUTOUT
}
private fun NExportKind.common(): SelectionExportKind = when (this) {
    NExportKind.MASK -> SelectionExportKind.Mask
    NExportKind.CUTOUT -> SelectionExportKind.Cutout
}
private fun SelectionEdit.native(): NEdit = NEdit(binding.native(), transactionId, deviceId, lamport, createdAtMs,
    when (val value = target) {
        is SelectionTarget.New -> NTarget.New(value.objectId, value.layerId)
        is SelectionTarget.Existing -> NTarget.Existing(value.selection.native())
    }, when (val value = operation) {
        is SelectionOperation.Rectangle -> NOperation.Rectangle(QueryRect(value.rectangle.x, value.rectangle.y,
            value.rectangle.width, value.rectangle.height), value.combine.native())
        is SelectionOperation.Lasso -> NOperation.Lasso(value.points.map { NPoint(it.x, it.y) }, value.combine.native())
        is SelectionOperation.Paint -> NOperation.Paint(value.points.map { NPoint(it.x, it.y) }, value.radius, value.opacity, value.combine.native())
        is SelectionOperation.Combine -> NOperation.Combine(value.other.native(), value.combine.native())
        SelectionOperation.Invert -> NOperation.Invert
        is SelectionOperation.Expand -> NOperation.Expand(value.radius)
        is SelectionOperation.Shrink -> NOperation.Shrink(value.radius)
        is SelectionOperation.Feather -> NOperation.Feather(value.radius)
    }, memoryBudgetBytes)

private fun mapSelectionFailure(error: SelectionException): SelectionFailure = when (error) {
    is SelectionException.Invalid -> SelectionFailure(SelectionFailureKind.Invalid, cause = error)
    is SelectionException.Conflict -> SelectionFailure(SelectionFailureKind.Conflict, cause = error)
    is SelectionException.ReusedTransaction -> SelectionFailure(SelectionFailureKind.ReusedTransaction, cause = error)
    is SelectionException.Locked -> SelectionFailure(SelectionFailureKind.Locked, cause = error)
    is SelectionException.Unsupported -> SelectionFailure(SelectionFailureKind.Unsupported, cause = error)
    is SelectionException.Corrupt -> SelectionFailure(SelectionFailureKind.Corrupt, cause = error)
    is SelectionException.Memory -> SelectionFailure(SelectionFailureKind.Memory, error.estimated, error.budget, error)
    is SelectionException.Limit -> SelectionFailure(SelectionFailureKind.Limit, cause = error)
    is SelectionException.EncodedLimit -> SelectionFailure(SelectionFailureKind.EncodedLimit, cause = error)
    is SelectionException.Storage -> SelectionFailure(SelectionFailureKind.Storage, cause = error)
    is SelectionException.Cancelled -> SelectionFailure(SelectionFailureKind.Cancelled, cause = error)
    is SelectionException.Closed -> SelectionFailure(SelectionFailureKind.Closed, cause = error)
    is SelectionException.Backpressure -> SelectionFailure(SelectionFailureKind.Backpressure, cause = error)
}
