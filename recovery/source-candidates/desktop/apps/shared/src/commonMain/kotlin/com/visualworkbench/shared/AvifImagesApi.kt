package com.visualworkbench.shared

/** Header-only admission reads no OS codec and never replaces the original.
 * Accepted source variants still require a present compatible OS decoder. */
public enum class AvifImageFailureKind { Invalid, Unavailable, Unsupported, Depth, Color, Orientation, Memory, Busy, Cancelled, Decode }
public class AvifImageFailure(public val kind:AvifImageFailureKind):Exception(kind.name)
public data class AvifColorInfo(val primaries:UShort,val transfer:UShort,val matrix:UShort,val fullRange:Boolean)
public data class AvifImageInfo(val sourceAssetId:String,val width:UInt,val height:UInt,val bitDepth:UByte,val orientation:UByte,val iccProfile:ByteArray,val nclx:AvifColorInfo?,val estimatedPeakBytes:ULong)
public expect fun inspectAvifImage(original:ByteArray,memoryBudgetBytes:ULong=256uL*1024uL*1024uL):AvifImageInfo
