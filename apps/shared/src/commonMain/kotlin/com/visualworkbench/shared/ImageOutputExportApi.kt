package com.visualworkbench.shared

/** Captured source D intent projected through the current canonical output frame.
 * Full exports retain a null region. Nonempty D selection/view rectangles are
 * intersected with the saved crop, quarter-turned and covered by integer O pixels.
 * Empty intersections fail instead of silently exporting the whole document. */
public data class ImageOutputExportGeometry(
    public val binding: WorkflowBinding,
    public val sourceAssetId: String,
    public val sourceWidth: UInt, public val sourceHeight: UInt,
    public val outputWidth: UInt, public val outputHeight: UInt,
    public val region: SelectionRegion?,
    public val encodedWidth: UInt, public val encodedHeight: UInt,
)

/** Reads metadata under the exact expected project/document/revision. A changed
 * binding fails Stale; it never consults an optional or cached frame panel.
 * Cancellation settles the native query before releasing the project borrow.
 * No original availability, color consent or preflight/export check is waived. */
public expect suspend fun imageOutputExportGeometry(
    project: WorkbenchProject, binding: WorkflowBinding, sourceRegion: SelectionRegion?,
    memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL,
): ImageOutputExportGeometry
