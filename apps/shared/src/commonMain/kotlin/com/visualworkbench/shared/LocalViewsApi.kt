package com.visualworkbench.shared

/** Installation identity is explicit and independent of a project's author.
 * These records are local settings; they never enter the canonical operation log.
 * Bookmarks do not encode a page. Compose them for image documents only until
 * the document owner admits a page-aware bookmark interface. */
public data class LocalViewsBinding(public val projectId:String,public val documentId:String,public val deviceId:String)
public data class RegionBookmark(public val id:String,public val name:String,public val camera:Camera)
public data class LocalViewsSnapshot(public val binding:LocalViewsBinding,public val revision:ULong,
    public val camera:Camera?,public val bookmarks:List<RegionBookmark>)
public sealed interface LocalViewsMutation {
    public data class Remember(public val camera:Camera):LocalViewsMutation
    public data class AddBookmark(public val id:String,public val name:String,public val camera:Camera):LocalViewsMutation
    public data class RenameBookmark(public val id:String,public val name:String):LocalViewsMutation
    public data class DeleteBookmark(public val id:String):LocalViewsMutation
}
/** Inject this port from the app-owned native facade; this leaf registers no FFI.
 * Borrowed from its project. Settle all calls before closing the project.
 * Memory is bounded to one <=16KiB settings row, <=32 names <=256 UTF8 bytes,
 * and 1MiB of transient work including generated/common DTO representations. */
public interface WorkbenchLocalViews {
    public suspend fun snapshot(binding:LocalViewsBinding,memoryBudgetBytes:ULong=1024uL*1024uL):LocalViewsSnapshot
    public suspend fun change(binding:LocalViewsBinding,expectedRevision:ULong,operationId:String,
        mutation:LocalViewsMutation,memoryBudgetBytes:ULong=1024uL*1024uL):LocalViewsSnapshot
}

internal fun validLocalCamera(camera:Camera):Boolean = listOf(camera.center.x,camera.center.y,camera.scale,camera.rotation,
    camera.viewportWidth,camera.viewportHeight).all{it.isFinite()} && kotlin.math.abs(camera.center.x)<=1e12 && kotlin.math.abs(camera.center.y)<=1e12 &&
    camera.scale in 1e-6..1e6 && kotlin.math.abs(camera.rotation)<=1e6 && camera.viewportWidth in 1.0..65536.0 && camera.viewportHeight in 1.0..65536.0
internal fun validBookmarkName(name:String):Boolean=name.isNotEmpty()&&name.length<=256&&name.encodeToByteArray().size<=256&&
    name.trim()==name&&name.none{it.code in 0..31||it.code in 127..159}
