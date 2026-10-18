package com.visualworkbench.shared

/** Full oriented original-grid pixels for the admitted local segmentation
 * adapter. This capability borrows the existing project worker and decoder;
 * neither obtaining it nor inspecting metadata decodes an image.
 *
 * The caller owns its actual project/currentness fence and checks it before and
 * after acquire. Native acquisition independently verifies the complete source
 * binding, dimensions and original bytes. Page documents and display proxies
 * are unsupported. The explicit untagged-color choice never changes originals. */
public interface WorkbenchAssistedSourcePixels {
    public suspend fun acquire(source: AssistedSelectionSource, assumeUntaggedSrgb: Boolean,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): AssistedSourcePixelsLease

    /** Retry cleanup of an undelivered acquisition before retiring its runtime
     * or borrowed project. A failed cleanup keeps that exact charged owner.
     * This does not close the project or a successfully delivered lease. */
    public suspend fun retireUndelivered(): Unit
}

/** Exclusive borrowed RGBA8 sRGB, top-left row-major, four bytes per oriented D
 * pixel. There are no annotations, tones, global output crop or resizing.
 * Consume pixels before close and do not retain the array elsewhere. A delivery
 * owns its native/ABI/foreign charges independently of the acquisition handle.
 * close joins retirement, is idempotent after success and retains failed owners;
 * no timeout or garbage collection is treated as successful retirement. */
public interface AssistedSourcePixelsLease {
    public val source: AssistedSelectionSource
    public val rgba: ByteArray
    public suspend fun close(): Unit
}

internal fun AssistedSelectionSource.requireAssistedOriginal(memoryBudgetBytes: ULong): Unit {
    admitShape(memoryBudgetBytes)
    if (page != null || document.sourceBitDepth !in setOf(8u, 16u))
        throw SelectionFailure(SelectionFailureKind.Unsupported)
    assistedSourceRgbaBytes()
}

internal fun AssistedSelectionSource.assistedSourceRgbaBytes(): Int {
    val pixels = document.width.toULong() * document.height.toULong()
    if (document.width == 0u || document.height == 0u || pixels > Int.MAX_VALUE.toULong() / 4uL)
        throw SelectionFailure(SelectionFailureKind.Limit)
    return (pixels * 4uL).toInt()
}

internal fun requireAssistedSourceReply(expected: AssistedSelectionSource,
    binding: SelectionBinding, width: UInt, height: UInt, sourceBitDepth: UInt,
    page: UInt?, rgbaBytes: ULong): Unit {
    if (binding != expected.binding || width != expected.document.width ||
        height != expected.document.height || sourceBitDepth != expected.document.sourceBitDepth ||
        page != expected.page || rgbaBytes != expected.assistedSourceRgbaBytes().toULong())
        throw SelectionFailure(SelectionFailureKind.Conflict)
}
