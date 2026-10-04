package com.visualworkbench.shared

public data class VectorEraseTarget(public val objectId: String, public val replacementId: String)
public data class VectorEraseEdit(
    public val binding: WorkflowBinding, public val metadata: WorkflowMetadata,
    public val targets: List<VectorEraseTarget>, public val centers: List<Point>, public val radius: Double,
    public val memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL,
    public val maxOutputVertices: UInt = 8_192u, public val maxWorkUnits: ULong = 64_000_000uL,
)
public data class VectorEraseReplacement(public val originalId: String, public val outlineId: String?, public val contourCount: UInt)
public data class VectorEraseReceipt(public val transactionId: String, public val changed: List<VectorEraseReplacement>,
    public val revision: ProjectInfo, public val estimatedPeakBytes: ULong)

/** Real D-space samples only. One saved transaction replaces affected raw
 * strokes with editable compound vector outlines; originals remain undoable.
 * All pieces share a NONZERO fill, preserving overlap alpha. The output is not
 * new raw pen input. Repeat erasing accepts canonical fill-only outlines.
 * Capture exact visible revision, targets and fresh replacement IDs once.
 * Cancellation settles native work; refresh for any already-durable commit.
 * Never replay automatically with regenerated IDs. Project owns the lifetime.
 */
public interface WorkbenchVectorEraser {
    public suspend fun erase(options: VectorEraseEdit): VectorEraseReceipt
}
public expect fun WorkbenchProject.vectorEraser(): WorkbenchVectorEraser

internal fun VectorEraseEdit.ownedEraseRequest(): VectorEraseEdit {
    if (targets.size !in 1..32 || centers.size !in 1..4_096 || maxOutputVertices !in 3u..262_144u || maxWorkUnits !in 1uL..64_000_000uL)
        throw WorkflowFailure(WorkflowFailureKind.Limit)
    if (!radius.isFinite() || radius !in 0.5..256.0 || memoryBudgetBytes !in 1uL..(256uL * 1024uL * 1024uL))
        throw WorkflowFailure(WorkflowFailureKind.Invalid)
    if (centers.any { !it.x.isFinite() || !it.y.isFinite() || kotlin.math.abs(it.x) > 1_000_000_000.0 || kotlin.math.abs(it.y) > 1_000_000_000.0 })
        throw WorkflowFailure(WorkflowFailureKind.Invalid)
    val ids = mutableSetOf<String>()
    for (target in targets) for (id in listOf(target.objectId, target.replacementId)) {
        if (id.length != 36 || !ids.add(id)) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    }
    return copy(targets = targets.map { it.copy() }, centers = centers.map { it.copy() })
}
