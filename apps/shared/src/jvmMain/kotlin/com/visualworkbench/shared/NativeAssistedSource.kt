package com.visualworkbench.shared

import com.visualworkbench.bindings.core.AssistedSourcePixels as NSource
import com.visualworkbench.bindings.core.AssistedSourcePixelsInfo as NInfo
import com.visualworkbench.bindings.core.SelectionBinding as NBinding
import com.visualworkbench.bindings.core.SelectionException
import com.visualworkbench.bindings.core.SessionResourceCharge as NCharge
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** Plain borrowed JVM/Android factory. The coordinator composes the existing
 * withNativeGiantProject lifetime borrow before generating this leaf's ABI. */
public fun assistedSourcePixels(project: WorkbenchProject): WorkbenchAssistedSourcePixels =
    NativeAssistedSource(project)

private class NativeAssistedSource(private val project: WorkbenchProject) : WorkbenchAssistedSourcePixels {
    // This serializes the capability's one undelivered owner, not project life.
    // Project close authority remains exclusively with withNativeGiantProject.
    private val gate: Mutex = Mutex()
    private var undelivered: NativeSourceOwner? = null

    override suspend fun acquire(source: AssistedSelectionSource, assumeUntaggedSrgb: Boolean,
        memoryBudgetBytes: ULong): AssistedSourcePixelsLease = gate.withLock {
        source.requireAssistedOriginal(memoryBudgetBytes)
        retirePending()
        val owner = NativeSourceOwner(source)
        undelivered = owner
        try {
            val result = withNativeGiantProject(project) { projectHandle ->
                settledWorkflow(release = { value: NativeSourceOwner -> value.releaseOwned() }) { cancellation ->
                    owner.handle = projectHandle.assistedSourcePixels(source.binding.native(), source.document.width,
                        source.document.height, source.document.sourceBitDepth, source.page, assumeUntaggedSrgb,
                        memoryBudgetBytes, cancellation)
                    owner.adoptPixels()
                    owner
                }
            }
            currentCoroutineContext().ensureActive()
            undelivered = null
            result
        } catch (error: Throwable) {
            withContext(NonCancellable) {
                try { retirePending() } catch (cleanup: Throwable) { error.addSuppressed(cleanup) }
            }
            throw sourceFailure(error)
        }
    }

    override suspend fun retireUndelivered(): Unit = withContext(NonCancellable) {
        gate.withLock { retirePending() }
    }

    private fun retirePending() {
        undelivered?.releaseOwned()
        undelivered = null
    }
}

private class NativeSourceOwner(override val source: AssistedSelectionSource) : AssistedSourcePixelsLease {
    var handle: NSource? = null
    private var charge: NCharge? = null
    private var deliveryReference: NCharge? = null
    private var pixels: ByteArray? = null
    private var closed: Boolean = false

    override val rgba: ByteArray
        @Synchronized get() = pixels ?: throw SelectionFailure(SelectionFailureKind.Closed)

    @Synchronized fun adoptPixels() {
        val native = handle ?: throw SelectionFailure(SelectionFailureKind.Closed)
        native.describe().check(source)
        // Acquire the opaque retirement authority BEFORE lifting the bulk
        // record. A failed ByteArray lift must not hide its last charge handle.
        charge = native.publicationLease()
        val delivery = native.takePixels()
        // Own the lease before examining the mutable foreign payload. Neither
        // validation nor cancellation can strand the full-grid charge.
        deliveryReference = delivery.nativeLease
        pixels = delivery.rgba
        if (delivery.rgba.size != source.assistedSourceRgbaBytes())
            throw SelectionFailure(SelectionFailureKind.Corrupt)
        releaseDeliveryReference()
        releaseHandle()
    }

    @Synchronized private fun releaseDeliveryReference() {
        // This is a second Arc of the already retained publication lease.
        // Drop only its generated handle, never its underlying reservation.
        deliveryReference?.let { it.destroy(); deliveryReference = null }
    }

    @Synchronized private fun releaseHandle() {
        val value = handle ?: return
        value.releaseSource()
        value.destroy()
        handle = null
    }

    @Synchronized fun releaseOwned() {
        if (closed) return
        try {
            releaseHandle()
            // All borrowers have retired before this method is called. Drop
            // this owner's bytes before crediting the delivered reservation.
            pixels = null
            releaseDeliveryReference()
            charge?.let { value -> value.releaseCharge(); value.destroy(); charge = null }
            closed = true
        } catch (error: Throwable) { throw sourceFailure(error) }
    }

    override suspend fun close(): Unit = withContext(NonCancellable) { releaseOwned() }
}

private fun SelectionBinding.native(): NBinding =
    NBinding(projectId, documentId, sourceAssetId, hostSeq, stateHash)

private fun NInfo.check(expected: AssistedSelectionSource) {
    requireAssistedSourceReply(expected, SelectionBinding(binding.projectId, binding.documentId,
        binding.sourceAssetId, binding.hostSeq, binding.stateHash), width, height,
        sourceBitDepth, page, rgbaBytes)
}

private fun sourceFailure(error: Throwable): Throwable = when (error) {
    is CancellationException, is SelectionFailure -> error
    is SelectionException -> when (error) {
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
    is GiantImageFailure -> when (error.kind) {
        GiantImageFailureKind.Closed -> SelectionFailure(SelectionFailureKind.Closed, cause = error)
        GiantImageFailureKind.Memory -> SelectionFailure(SelectionFailureKind.Memory, error.estimatedBytes, error.budgetBytes, error)
        GiantImageFailureKind.Stale -> SelectionFailure(SelectionFailureKind.Conflict, cause = error)
        GiantImageFailureKind.Unsupported -> SelectionFailure(SelectionFailureKind.Unsupported, cause = error)
        GiantImageFailureKind.Storage -> SelectionFailure(SelectionFailureKind.Storage, cause = error)
        GiantImageFailureKind.Backpressure -> SelectionFailure(SelectionFailureKind.Backpressure, cause = error)
        GiantImageFailureKind.Cancelled -> SelectionFailure(SelectionFailureKind.Cancelled, cause = error)
        GiantImageFailureKind.Limit -> SelectionFailure(SelectionFailureKind.Limit, cause = error)
        else -> SelectionFailure(SelectionFailureKind.Invalid, cause = error)
    }
    is WorkflowFailure -> when (error.kind) {
        WorkflowFailureKind.Closed, WorkflowFailureKind.Detached -> SelectionFailure(SelectionFailureKind.Closed, cause = error)
        WorkflowFailureKind.Memory -> SelectionFailure(SelectionFailureKind.Memory, error.estimatedBytes, error.budgetBytes, error)
        WorkflowFailureKind.Backpressure -> SelectionFailure(SelectionFailureKind.Backpressure, cause = error)
        WorkflowFailureKind.Cancelled -> SelectionFailure(SelectionFailureKind.Cancelled, cause = error)
        WorkflowFailureKind.Storage -> SelectionFailure(SelectionFailureKind.Storage, cause = error)
        WorkflowFailureKind.Stale -> SelectionFailure(SelectionFailureKind.Conflict, cause = error)
        WorkflowFailureKind.Limit -> SelectionFailure(SelectionFailureKind.Limit, cause = error)
        else -> SelectionFailure(SelectionFailureKind.Invalid, cause = error)
    }
    is IllegalStateException -> SelectionFailure(SelectionFailureKind.Closed, cause = error)
    else -> error
}
