package com.visualworkbench.shared

/** Optional capability; obtaining this interface never starts a producer or
 * substitutes a derived proxy for the authoritative original. Close all images
 * and join renderer retirement before closing/replacing the project. */
public interface WorkbenchGiantImagesProject {
    public suspend fun openGiantImage(binding: WorkflowBinding, diskCacheBytes: ULong,
        ramCacheBytes: ULong): GiantImage
}
public data class GiantViewport(public val left: Double, public val top: Double,
    public val right: Double, public val bottom: Double,
    public val a: Double, public val b: Double, public val c: Double, public val d: Double)
public data class GiantTileAddress(public val level: UByte, public val x: UInt, public val y: UInt)
public data class GiantViewportPlan(public val assetId: String, public val manifestHash: String,
    public val level: UByte, public val visible: List<GiantTileAddress>, public val prefetch: List<GiantTileAddress>)
/** Owned immutable-by-contract derivative. Never log or stringify pixel bytes. */
public class GiantTilePixels(public val assetId: String, public val manifestHash: String,
    public val address: GiantTileAddress, public val documentX: UInt, public val documentY: UInt,
    public val documentWidth: UInt, public val documentHeight: UInt,
    public val width: UInt, public val height: UInt, public val rgba: ByteArray) {
    public override fun toString(): String = "GiantTilePixels(width=$width, height=$height)"
}
/** Each cancellation settles the actual bounded native read before propagating.
 * Source/manifest identity cannot change within a handle. close joins native
 * reads and releases its own cache owners; renderer retirement is a distinct
 * caller obligation and a failed owner must stay retained and charged. */
public interface GiantImage {
    public suspend fun viewport(view: GiantViewport): GiantViewportPlan
    /** The live reservation covers foreign bytes/bitmap/texture copies before
     * native admission. Keep it through actual renderer retirement. */
    public suspend fun tile(address: GiantTileAddress, foreign: SessionResourceCharge): GiantTilePixels
    public suspend fun close(): Unit
}
public enum class GiantImageFailureKind { Invalid, Limit, Memory, Stale, Missing, Unsupported, Closed, Cancelled, Backpressure, Storage }
public class GiantImageFailure(public val kind: GiantImageFailureKind,
    public val estimatedBytes: ULong? = null, public val budgetBytes: ULong? = null,
    cause: Throwable? = null) : Exception(kind.name, cause)
