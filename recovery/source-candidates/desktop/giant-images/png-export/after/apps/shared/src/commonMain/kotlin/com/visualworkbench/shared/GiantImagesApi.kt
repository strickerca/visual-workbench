package com.visualworkbench.shared

/** Paths are app-private producer inputs, never export originals or provider URIs. */
public data class GiantInputOptions(public val sourcePath:String,public val workDirectory:String,public val maxOriginalBytes:ULong=4uL*1024uL*1024uL*1024uL)
public data class GiantInputInfo(public val sourcePath:String,public val assetId:String,public val sha256:String,public val encodedBytes:ULong,public val rawWidth:UInt,public val rawHeight:UInt,public val width:UInt,public val height:UInt,public val orientation:UByte,public val format:String)
public data class GiantProjectOptions(public val path:String,public val projectId:String,public val documentId:String,public val layerId:String,public val deviceId:String,public val title:String,public val nowMs:Long,public val diskCacheBytes:ULong)
/** D-space inverse viewport envelope plus the D-to-physical affine linear part. */
public data class GiantViewport(public val left:Double,public val top:Double,public val right:Double,public val bottom:Double,public val a:Double,public val b:Double,public val c:Double,public val d:Double)
public data class GiantTileAddress(public val level:UByte,public val x:UInt,public val y:UInt)
public data class GiantViewportPlan(public val assetId:String,public val manifestHash:String,public val level:UByte,public val visible:List<GiantTileAddress>,public val prefetch:List<GiantTileAddress>)
/** Straight-alpha opaque RGBA8 sRGB display pixels. These are derived cache data.
 * Caller reserves foreign bitmap/texture bytes before querying and releases them
 * on viewport retirement. Native cache accounting does not include foreign copies. */
public data class GiantTilePixels(public val assetId:String,public val manifestHash:String,public val address:GiantTileAddress,public val documentX:UInt,public val documentY:UInt,public val documentWidth:UInt,public val documentHeight:UInt,public val width:UInt,public val height:UInt,public val rgba:ByteArray)
public interface GiantPreparedSource {
    public suspend fun describe():GiantInputInfo
    /** Only the approved platform decoder's completed private output is accepted.
     * An arbitrary external pyramid is not a supported import source. */
    public suspend fun publish(options:GiantProjectOptions,approvedProducerDirectory:String):WorkbenchProject
    public suspend fun rebuild(project:WorkbenchProject,documentId:String,approvedProducerDirectory:String,diskCacheBytes:ULong)
    /** Must be joined after the platform producer settles, including cancellation. */
    public suspend fun close()
}
public interface GiantImage {
    public suspend fun viewport(view:GiantViewport):GiantViewportPlan
    public suspend fun tile(address:GiantTileAddress):GiantTilePixels
    /** Base lossless tiles only; concurrent tile/export work returns Backpressure. */
    public suspend fun exportPng(options:GiantPngOptions):GiantPngReceipt
    /** Joins all admitted native reads before the project can close. */
    public suspend fun close()
}
public expect suspend fun prepareGiantImage(options:GiantInputOptions):GiantPreparedSource
public expect suspend fun WorkbenchProject.openGiantImage(documentId:String,diskCacheBytes:ULong,ramCacheBytes:ULong):GiantImage

public expect suspend fun WorkbenchProject.prepareGiantRebuild(documentId:String,workDirectory:String):GiantPreparedSource
