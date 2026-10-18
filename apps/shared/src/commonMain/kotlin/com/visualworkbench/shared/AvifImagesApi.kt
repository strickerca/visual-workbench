package com.visualworkbench.shared

/** Header-only admission reads no OS codec and never replaces the original.
 * Accepted source variants still require a present compatible OS decoder. */
public enum class AvifImageFailureKind { Invalid, Unavailable, Unsupported, Depth, Color, Orientation, Memory, Busy, Cancelled, Decode }
public class AvifImageFailure(public val kind:AvifImageFailureKind):Exception(kind.name)
public data class AvifColorInfo(public val primaries:UShort,public val transfer:UShort,public val matrix:UShort,public val fullRange:Boolean)
public class AvifImageInfo(public val sourceAssetId:String,public val width:UInt,public val height:UInt,public val bitDepth:UByte,public val orientation:UByte,public val iccProfile:ByteArray,public val nclx:AvifColorInfo?,public val estimatedPeakBytes:ULong) {
    public override fun toString():String = "AvifImageInfo(width=$width, height=$height, bitDepth=$bitDepth)"
}
public expect fun inspectAvifImage(original:ByteArray,memoryBudgetBytes:ULong=256uL*1024uL*1024uL):AvifImageInfo
