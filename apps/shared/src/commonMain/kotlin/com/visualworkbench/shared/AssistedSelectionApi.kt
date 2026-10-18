package com.visualworkbench.shared

/** The native source content hash and full visible revision travel together.
 * Page and attachment epoch also fence same-sized pages and reopened projects. */
public data class AssistedSelectionSource(public val document: SelectionDocument,
    public val page: UInt?, public val attachmentEpoch: Long) {
    public val binding: SelectionBinding get() = document.binding
    public val workflowBinding: WorkflowBinding get() = WorkflowBinding(binding.projectId,
        binding.documentId, binding.hostSeq, binding.stateHash)
}

/** Closed pins supplied only after the model/runtime license and artifact gates
 * pass. There are deliberately no production defaults or download locations. */
public data class AssistedSelectionPins(public val modelArtifactSha256: String,
    public val runtimeArtifactSha256: String, public val modelLicenseAdmissionId: String,
    public val runtimeLicenseAdmissionId: String, public val runtimeAdmissionId: String)

public enum class AssistedSelectionExecution { Npu, Gpu, Cpu }

/** Trusted local adapter boundary. admit must hash the actual local artifacts,
 * check their CLOSED license/runtime admissions and inspect actual capabilities
 * before returning. This step is bounded artifact verification/capability
 * inspection only: it MUST NOT load a model, decode source pixels or allocate
 * embeddings/scratch before the owner's working-set reservation in encode.
 * It must settle cancellation and retire partial owners when
 * throwing. No adapter is supplied until those external decisions are closed. */
public interface AssistedSelectionBackend {
    public suspend fun admit(pins: AssistedSelectionPins): AssistedSelectionRuntime
}

public interface AssistedSelectionRuntime {
    /** Observed, verified identities, not values copied from an unchecked request. */
    public val admittedPins: AssistedSelectionPins
    /** Only execution paths implemented by this admitted local runtime. */
    public val executions: Set<AssistedSelectionExecution>
    /** Conservative bound for model residency, source decode, encoding, retained
     * embeddings and decoder scratch; inspected before any such allocations. */
    public val workingSetBytes: ULong
    /** Encode this exact native source once. Verify its asset hash/oriented
     * dimensions; refuse a page/revision mismatch instead of substituting pixels.
     * The owner has reserved workingSetBytes before entry. Failure/cancellation
     * must settle work and release partial session owners before throwing. */
    public suspend fun encode(source: AssistedSelectionSource,
        execution: AssistedSelectionExecution): AssistedSelectionSession
    /** Return only after all native workers/handles have actually retired.
     * Failure retains ownership and permits a later close attempt. */
    public suspend fun close(): Unit
}

/** One encoded image. Prompt evaluation reuses its actual embeddings; a CPU
 * fallback is selected only from the admitted runtime's executions. There is
 * one bounded CPU initialization fallback after a settled Unsupported result
 * from the preferred admitted execution path. Prompt inference is never retried. */
public interface AssistedSelectionSession {
    public val source: AssistedSelectionSource
    public val execution: AssistedSelectionExecution
    /** Transfer exclusive output-byte ownership to the caller. Return full
     * oriented-document 8-bit coverage within the supplied source pixel budget.
     * Cancellation must join the real producer before returning or throwing. */
    public suspend fun propose(request: AssistedSelectionPromptRequest): AssistedSelectionMask
    public suspend fun close(): Unit
}

public data class AssistedSelectionPrompt(public val positive: List<Point> = emptyList(),
    public val negative: List<Point> = emptyList(), public val box: Rect? = null)
public data class AssistedSelectionPromptRequest(public val source: AssistedSelectionSource,
    public val sequence: ULong, public val prompt: AssistedSelectionPrompt)
public data class AssistedSelectionMask(public val source: AssistedSelectionSource,
    public val sequence: ULong, public val width: UInt, public val height: UInt,
    public val coverage: ByteArray)

/** Immutable adopted proposal. Public previews are bounded coverage tiles, using
 * the same document coordinates as existing selection overlays. */
public class AssistedSelectionProposal internal constructor(public val source: AssistedSelectionSource,
    public val sequence: ULong, internal val ownedCoverage: ByteArray) {
    public fun tile(region: SelectionRegion): SelectionTile {
        if (region.width !in 1u..256u || region.height !in 1u..256u ||
            region.x.toULong() + region.width > source.document.width.toULong() ||
            region.y.toULong() + region.height > source.document.height.toULong())
            throw SelectionFailure(SelectionFailureKind.Invalid)
        val result = ByteArray((region.width * region.height).toInt())
        repeat(region.height.toInt()) { row ->
            val start = ((region.y.toULong() + row.toULong()) * source.document.width + region.x).toInt()
            ownedCoverage.copyInto(result, row * region.width.toInt(), start, start + region.width.toInt())
        }
        return SelectionTile(region, result)
    }
}

public enum class AssistedSelectionPhase { Unavailable, Ready, Encoding, Proposing, Proposal, Saving, Saved, Stale, Failed, Closed }
public data class AssistedSelectionState(public val phase: AssistedSelectionPhase,
    public val proposal: AssistedSelectionProposal? = null,
    public val execution: AssistedSelectionExecution? = null,
    public val receipt: SelectionReceipt? = null,
    public val issue: String? = null,
    /** Preserve the exact existing selection failure, including memory details. */
    public val selectionFailure: SelectionFailure? = null,
) {
    public val canPrompt: Boolean get() = phase in setOf(AssistedSelectionPhase.Ready,
        AssistedSelectionPhase.Encoding, AssistedSelectionPhase.Proposing, AssistedSelectionPhase.Proposal)
    public val canAccept: Boolean get() = phase == AssistedSelectionPhase.Proposal && proposal != null
}

internal fun AssistedSelectionPins.admitShape(): Unit {
    fun hash(value: String): Boolean = value.length == 64 && value.all { it in "0123456789abcdef" }
    fun identity(value: String): Boolean = value.length in 1..128 && value.all { it.code in 33..126 }
    if (!hash(modelArtifactSha256) || !hash(runtimeArtifactSha256) ||
        !identity(modelLicenseAdmissionId) || !identity(runtimeLicenseAdmissionId) || !identity(runtimeAdmissionId))
        throw SelectionFailure(SelectionFailureKind.Invalid)
}

internal fun AssistedSelectionSource.admitShape(memoryBudgetBytes: ULong): Unit {
    if (attachmentEpoch < 0 || binding.projectId.length !in 1..128 || binding.documentId.length !in 1..128 ||
        binding.sourceAssetId.length != 64 || binding.sourceAssetId.any { it !in "0123456789abcdef" } ||
        binding.stateHash.length != 64 || binding.stateHash.any { it !in "0123456789abcdef" } ||
        !binding.matches(document.revision, binding.documentId)) throw SelectionFailure(SelectionFailureKind.Invalid)
    val pixels = document.width.toULong() * document.height.toULong()
    if (pixels > Int.MAX_VALUE.toULong()) throw SelectionFailure(SelectionFailureKind.Limit)
    admitSelectionCoverage(document.width, document.height, pixels.toInt(), memoryBudgetBytes)
}

internal fun AssistedSelectionPrompt.owned(source: AssistedSelectionSource): AssistedSelectionPrompt {
    // Count and geometry checks precede all list copies. No truncation/prediction.
    if (positive.size > 64 || negative.size > 64 || positive.size + negative.size > 64 ||
        (positive.isEmpty() && box == null)) throw SelectionFailure(SelectionFailureKind.Limit)
    fun point(p: Point): Boolean = p.x.isFinite() && p.y.isFinite() &&
        p.x >= 0.0 && p.y >= 0.0 && p.x < source.document.width.toDouble() && p.y < source.document.height.toDouble()
    if (positive.any { !point(it) } || negative.any { !point(it) }) throw SelectionFailure(SelectionFailureKind.Invalid)
    box?.let {
        if (!it.x.isFinite() || !it.y.isFinite() || !it.width.isFinite() || !it.height.isFinite() ||
            it.x < 0.0 || it.y < 0.0 || it.width <= 0.0 || it.height <= 0.0 ||
            it.x + it.width > source.document.width.toDouble() || it.y + it.height > source.document.height.toDouble())
            throw SelectionFailure(SelectionFailureKind.Invalid)
    }
    return copy(positive = positive.toList(), negative = negative.toList())
}
