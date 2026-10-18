package com.visualworkbench.shared

/** Exact original-resolution PNG8. The initial giant producer supports opaque
 * untagged 8-bit originals only; no hidden profile/depth conversion is offered.
 * Integer region coordinates are in the original oriented document space. */
public data class GiantPngOptions(
    public val binding:WorkflowBinding,
    public val outputPath:String,
    public val marked:Boolean=true,
    public val region:Rect?=null,
    public val assumeUntaggedSrgb:Boolean=false,
    public val memoryBudgetBytes:ULong=256uL*1024uL*1024uL,
    public val maxEncodedBytes:ULong=4uL*1024uL*1024uL*1024uL,
)
public data class GiantPngReceipt(
    public val revision:ProjectInfo,
    public val encodedBytes:ULong,
    public val blake3:String,
    public val metadataJson:String,
    /** Accounted native source/snapshot/strip working set, not measured RSS. */
    public val estimatedPeakBytes:ULong,
)
