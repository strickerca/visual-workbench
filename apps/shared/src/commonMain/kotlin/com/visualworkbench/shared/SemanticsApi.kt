package com.visualworkbench.shared

public enum class SemanticPlatform { Uia, AndroidAx, ChromiumUia }
public enum class SemanticBoundsSpace { HostPhysical, CapturePixels }
/** Provider observations are bounded untrusted data. Do not mutate supplied
 * collections after a call begins. Android adapters normalize display rotation
 * into screenshot pixels before choosing CapturePixels. */
public data class CapturedSemanticElement(public val localId: String, public val parentLocalId: String?, public val name: String, public val role: String, public val automationId: String?, public val resourceId: String?, public val htmlId: String?, public val bounds: Rect, public val text: String, public val enabled: Boolean, public val focused: Boolean)
public data class SemanticElement(public val eid: String, public val parent: String?, public val name: String, public val role: String, public val automationId: String?, public val resourceId: String?, public val htmlId: String?, public val boundsDocument: Rect, public val boundsClipped: Boolean, public val text: String, public val enabled: Boolean, public val focused: Boolean)
public data class SemanticCaptureInfo(public val binding: WorkflowBinding, public val captureSessionId: String, public val frameId: ULong, public val geometryRevision: UInt, public val sourceAssetId: String, public val capturedAtMs: Long, public val width: Double, public val height: Double)
public data class SemanticDocument(public val binding: WorkflowBinding, public val snapshotId: String, public val platform: SemanticPlatform, public val frameDeltaMs: Int, public val collectionElapsedMs: ULong, public val elements: List<SemanticElement>)
public sealed interface SemanticSnapQuery {
    public data class PointQuery(public val point: Point) : SemanticSnapQuery
    public data class BoxQuery(public val bounds: Rect) : SemanticSnapQuery
}
public data class SemanticSnap(public val snapshotId: String, public val binding: WorkflowBinding, public val eid: String, public val boundsDocument: Rect, public val distanceScreenPixels: Double)
public data class SemanticReference(public val platform: SemanticPlatform, public val eid: String, public val name: String, public val role: String, public val automationId: String?, public val resourceId: String?, public val htmlId: String?, public val boundsDocument: Rect)
public data class SemanticProjection(public val binding: WorkflowBinding, public val snapshotId: String, public val references: List<SemanticReference>, public val semanticJson: ByteArray, public val quotedPromptData: String)
/** Begin before OS collection; the ticket retains the complete private capture
 * context. A new frame, geometry or any visible revision refuses late results.
 * Source windows, owner consent, own-app exclusion and screenshot collection are
 * provider responsibilities. Frame delta and elapsed time are observed inputs,
 * never claims that a capture/300ms platform acceptance test passed. */
public interface WorkbenchSemanticCollection {
    public fun describe(): SemanticCaptureInfo
    public suspend fun prepare(metadata: WorkflowMetadata, snapshotId: String, platform: SemanticPlatform, frameDeltaMs: Int, collectionElapsedMs: ULong, boundsSpace: SemanticBoundsSpace, elements: List<CapturedSemanticElement>): WorkbenchWorkflowPlan
    public fun close()
}
public interface WorkbenchSemantics {
    public suspend fun beginCapture(binding: WorkflowBinding, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchSemanticCollection
    public suspend fun document(binding: WorkflowBinding, snapshotId: String, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): SemanticDocument
    /** The full D-to-physical-screen transform includes camera, rotation/insets;
     * the native library enforces its canonical12physical-pixel snap threshold. */
    public suspend fun snap(binding: WorkflowBinding, snapshotId: String, query: SemanticSnapQuery, documentToScreen: Transform, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): SemanticSnap?
    /** null exports all elements; an explicit list requires exact EID resolution.
     * Window title/native handles/device identity never enter this projection. */
    public suspend fun export(binding: WorkflowBinding, snapshotId: String, elementEids: List<String>? = null, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): SemanticProjection
}
public expect fun semanticWorkflows(project: WorkbenchProject): WorkbenchSemantics
