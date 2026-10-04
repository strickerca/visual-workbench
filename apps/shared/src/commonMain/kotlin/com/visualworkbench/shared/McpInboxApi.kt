package com.visualworkbench.shared

/** Untrusted external return, retained only for comparison. It carries no AI
 * exterior-pixel proof and never changes the open document on arrival. */
public data class McpInboxSubmission(public val receiptId:String,public val packageId:String,public val target:String,
    public val manifestSha256:String,public val connection:String,public val createdAtMs:Long,
    public val text:String?=null,public val png:ByteArray?=null,public val note:String="")
public data class McpInboxImage(public val width:UInt,public val height:UInt,public val bitDepth:UByte,
    public val encodedBytes:ULong,public val blake3:String,public val iccBlake3:String?,public val orientationApplied:UByte)
public data class McpInboxReceipt(public val receiptId:String,public val receiptBlake3:String,public val packageId:String,
    public val target:String,public val manifestSha256:String,public val binding:WorkflowBinding,public val sourceAssetId:String,
    public val connection:String,public val createdAtMs:Long,public val text:String?,public val note:String,
    public val before:McpInboxImage,public val after:McpInboxImage?,public val retired:Boolean)
public enum class McpInboxSide { Before, After }
public data class McpInboxRegion(public val x:UInt,public val y:UInt,public val width:UInt,public val height:UInt)
public data class McpInboxPixels(public val receiptId:String,public val receiptBlake3:String,public val side:McpInboxSide,
    public val region:McpInboxRegion,public val canvasWidth:UInt,public val canvasHeight:UInt,public val rgbaSrgb:ByteArray)
public data class McpInboxRetirement(public val receiptId:String,public val receiptBlake3:String,public val filesRemoved:Boolean)
public interface WorkbenchMcpInbox {
    /** The caller owns a new receipt ID before requesting persistence. Exact ID
     * retries recover lost delivery; this does not deduplicate separate tool calls. */
    public suspend fun submit(catalog:WorkbenchPackageCatalog,submission:McpInboxSubmission):McpInboxReceipt
    public suspend fun list():List<McpInboxReceipt>
    /** Exact original encoded PNG, bounded to4MiB. No display conversion. */
    public suspend fun readPng(receiptId:String,receiptBlake3:String,side:McpInboxSide):ByteArray
    /** At most512x512 straight-alpha RGBA sRGB. Before is the immutable package
     * clean image at its package resolution, never a newer project background. */
    public suspend fun pixels(receiptId:String,receiptBlake3:String,side:McpInboxSide,region:McpInboxRegion,
        assumeUntaggedSrgb:Boolean=false,allowDepthReduction:Boolean=false):McpInboxPixels
    /** Caller joins its tile/export consumers first. Unsupported platform cleanup
     * reports filesRemoved=false and retains bytes/quota after logical retirement. */
    public suspend fun retire(receiptId:String,receiptBlake3:String):McpInboxRetirement
    public suspend fun close()
}
public expect suspend fun openMcpResultInbox(applicationPrivateDirectory:String):WorkbenchMcpInbox
