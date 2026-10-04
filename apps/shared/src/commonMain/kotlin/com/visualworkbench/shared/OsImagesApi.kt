package com.visualworkbench.shared

/** Header-only admission reads no OS codec and never replaces the original.
 * Accepted source variants still require a present compatible OS decoder. */
public enum class OsImageFailureKind { Invalid, Unavailable, Unsupported, Depth, Color, Orientation, Memory, Busy, Cancelled, Decode }
public class OsImageFailure(public val kind:OsImageFailureKind):Exception(kind.name)
public data class OsImageInfo(val sourceAssetId:String,val width:UInt,val height:UInt,val bitDepth:UByte,val orientation:UByte,val iccProfile:ByteArray,val estimatedPeakBytes:ULong)
public expect fun inspectOsImage(original:ByteArray,memoryBudgetBytes:ULong=256uL*1024uL*1024uL):OsImageInfo
