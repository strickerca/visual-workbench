package com.visualworkbench.shared

/** Provider observations. Creation hashes and retains the exact PNG original,
 * validates the decoded extent, and publishes the capture document atomically.
 * Creating this DTO does not itself authorize capture or collect semantics. */
public data class CaptureImportDescriptor(
    public val explicitOwnerAction:Boolean,
    public val captureSessionId:String,
    public val frameId:ULong,
    public val geometryRevision:UInt,
    public val platform:String,
    public val sourceKind:String,
    public val physicalX:Int,
    public val physicalY:Int,
    public val width:UInt,
    public val height:UInt,
    public val dpiScale:Double,
    public val monotonicTimestampNs:ULong,
    public val capturedAtMs:Long,
    public val windowHandle:ULong=0uL,
    public val monitorId:String="",
    public val expectedSourceAssetId:String?=null,
)
/** Settles native import cancellation before releasing the caller's source or
 * scratch ownership. The caller closes the returned project after adoption. */
public expect suspend fun createCaptureProject(options:CreateFileProject,capture:CaptureImportDescriptor):WorkbenchProject
