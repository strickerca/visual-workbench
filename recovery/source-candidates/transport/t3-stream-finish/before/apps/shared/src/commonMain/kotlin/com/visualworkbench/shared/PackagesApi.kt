package com.visualworkbench.shared

public enum class PackageFailureKind { Invalid, Limit, Stale, Original, Depth, Unsupported, Integrity, Identity, Storage, Busy, Closed, Cancelled }
public class PackageFailure(public val kind:PackageFailureKind):Exception("Package: ${kind.name}")
/** A model profile is an explicit owner choice. This does not assert that an
 * installed client supports local images; Codex needs a verified runtime adapter. */
public sealed interface PackageTarget {
    public data class ClaudeModern(public val model:String):PackageTarget
    public data class ClaudeLegacy(public val model:String):PackageTarget
    public data class OpenAiResponses(public val model:String,public val maxLongEdge:UInt):PackageTarget
    public data class Gemini(public val model:String,public val maxLongEdge:UInt):PackageTarget
    public data class Generic(public val maxLongEdge:UInt=2048u):PackageTarget
}
public data class PackageCompileOptions(public val binding:WorkflowBinding,public val packageId:String,
    public val createdAtMs:Long,public val target:PackageTarget,public val semanticSnapshotId:String?=null,
    public val includeWindowTitle:Boolean=false,public val assumeUntaggedSrgb:Boolean=false,
    public val allowDepthReduction:Boolean=false,public val memoryBudgetBytes:ULong=256uL*1024uL*1024uL)
public data class PackageImageInfo(public val id:String,public val role:String,public val path:String,
    public val width:UInt,public val height:UInt,public val encodedBytes:ULong)
public data class PackageInfo(public val packageId:String,public val target:String,public val model:String?,
    public val binding:WorkflowBinding,public val sourceAssetId:String,public val manifestSha256:String,
    public val createdAt:String,public val totalBytes:ULong,public val markerCount:UInt,
    public val images:List<PackageImageInfo>,public val inlineImagesAvailable:Boolean,public val includesWindowTitle:Boolean)
public data class PublishedPackage(public val info:PackageInfo,public val directory:String)
public data class PackageRetirement(public val packageId:String,public val target:String,
    public val manifestSha256:String,public val filesRemoved:Boolean)
/** Compilation is immutable and never changes the document, grants capture or sends a message. */
public interface WorkbenchCompiledPackage {
    public val info:PackageInfo
    public suspend fun close()
}
/** Private app-owned catalog; all cancellation paths settle native work before
 * returning. Publication may have committed when cancellation is delivered: list
 * and the exact package ID/hash provide recovery, never an automatic new ID. */
public interface WorkbenchPackageCatalog {
    public suspend fun publish(packageValue:WorkbenchCompiledPackage):PublishedPackage
    public suspend fun list():List<PublishedPackage>
    public suspend fun lookup(packageId:String,target:String,manifestSha256:String):PublishedPackage
    public suspend fun readFile(packageId:String,target:String,manifestSha256:String,name:String,maxBytes:UInt):ByteArray
    public suspend fun pendingRetirement():PackageRetirement?
    /** First unpublish and await the MCP drain, then release UI/drag leases.
     * The returned receipt distinguishes durable unpublishing from file cleanup.
     * A pending cleanup can be retried using the same exact receipt after reopen. */
    public suspend fun retire(packageId:String,target:String,manifestSha256:String):PackageRetirement
    public suspend fun close()
}
public expect suspend fun WorkbenchProject.compilePackage(options:PackageCompileOptions):WorkbenchCompiledPackage
/** The directory must already exist, be private to this app, and be outside a project. */
public expect suspend fun openPackageCatalog(applicationPrivateDirectory:String):WorkbenchPackageCatalog
