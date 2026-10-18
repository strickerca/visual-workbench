package com.visualworkbench.shared

import com.visualworkbench.bindings.core.GiantInput as NInput
import com.visualworkbench.bindings.core.GiantInputOptions as NInputOptions
import com.visualworkbench.bindings.core.GiantProjectOptions as NProjectOptions
import com.visualworkbench.bindings.core.GiantImage as NImage
import com.visualworkbench.bindings.core.GiantViewport as NViewport
import com.visualworkbench.bindings.core.GiantTileAddress as NAddress
import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.prepareGiantImage as prepareNative
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Every concurrent/cancelled closer observes the same completed shutdown. */
internal class GiantHandleOwner {
    private val closed=AtomicBoolean(false)
    private val gate=Mutex()
    private var settled:Result<Unit>?=null
    fun checkOpen(){if(closed.get())throw CoreFailure(CoreFailureKind.Closed)}
    suspend fun close(release:suspend()->Unit){closed.set(true);withContext(NonCancellable){gate.withLock{val result=settled?:runCatching{release()}.also{settled=it};result.getOrThrow()}}}
}
public actual suspend fun prepareGiantImage(options:GiantInputOptions):GiantPreparedSource {
    val handle=settledStream(release={value:NInput->try{value.shutdown()}finally{value.destroy()}}){cancel->prepareNative(NInputOptions(options.sourcePath,options.workDirectory,options.maxOriginalBytes),cancel)}
    try{return NativeGiantInput(handle)}catch(error:Throwable){releaseNative{try{handle.shutdown()}finally{handle.destroy()}};throw error}
}
private class NativeGiantInput(private val handle:NInput):GiantPreparedSource {
    private val owner=GiantHandleOwner()
    override suspend fun describe():GiantInputInfo {owner.checkOpen();return settledStream{handle.describe().let{GiantInputInfo(it.sourcePath,it.assetId,it.sha256,it.encodedBytes,it.rawWidth,it.rawHeight,it.width,it.height,it.orientation,it.format)}}}
    override suspend fun publish(options:GiantProjectOptions,approvedProducerDirectory:String):WorkbenchProject {
        owner.checkOpen()
        val project=settledStream(release={value:ProjectSession->try{value.closeSession()}finally{value.destroy()}}){cancel->handle.publishProject(NProjectOptions(options.path,options.projectId,options.documentId,options.layerId,options.deviceId,options.title,options.nowMs,options.diskCacheBytes),approvedProducerDirectory,cancel)}
        try{return wrapNativeProject(project)}catch(error:Throwable){releaseNative{try{project.closeSession()}finally{project.destroy()}};throw error}
    }
    override suspend fun rebuild(project:WorkbenchProject,documentId:String,approvedProducerDirectory:String,diskCacheBytes:ULong){owner.checkOpen();settledStream{cancel->handle.rebuildCache(nativeProjectHandle(project),documentId,approvedProducerDirectory,diskCacheBytes,cancel)}}
    override suspend fun close(){owner.close{try{handle.shutdown()}finally{handle.destroy()}}}
}
public actual suspend fun WorkbenchProject.prepareGiantRebuild(documentId:String,workDirectory:String):GiantPreparedSource {
    val borrowed=nativeProjectHandle(this)
    val handle=settledStream(release={value:NInput->try{value.shutdown()}finally{value.destroy()}}){cancel->borrowed.prepareGiantRebuild(documentId,workDirectory,cancel)}
    try{return NativeGiantInput(handle)}catch(error:Throwable){releaseNative{try{handle.shutdown()}finally{handle.destroy()}};throw error}
}
public actual suspend fun WorkbenchProject.openGiantImage(documentId:String,diskCacheBytes:ULong,ramCacheBytes:ULong):GiantImage {
    val borrowed=nativeProjectHandle(this)
    val handle=settledStream(release={value:NImage->try{value.shutdown()}finally{value.destroy()}}){cancel->borrowed.openGiantImage(documentId,diskCacheBytes,ramCacheBytes,cancel)}
    try{return NativeGiantImage(handle)}catch(error:Throwable){releaseNative{try{handle.shutdown()}finally{handle.destroy()}};throw error}
}
private class NativeGiantImage(private val handle:NImage):GiantImage {
    private val owner=GiantHandleOwner()
    override suspend fun exportPng(options:GiantPngOptions):GiantPngReceipt {owner.checkOpen();return nativeGiantPng(handle,options)}
    override suspend fun viewport(view:GiantViewport):GiantViewportPlan {owner.checkOpen();return settledStream{handle.viewport(NViewport(view.left,view.top,view.right,view.bottom,view.a,view.b,view.c,view.d)).let{GiantViewportPlan(it.assetId,it.manifestHash,it.level,it.visible.map{key->GiantTileAddress(key.level,key.x,key.y)},it.prefetch.map{key->GiantTileAddress(key.level,key.x,key.y)})}}}
    override suspend fun tile(address:GiantTileAddress):GiantTilePixels {owner.checkOpen();return settledStream{cancel->handle.tile(NAddress(address.level,address.x,address.y),cancel).let{GiantTilePixels(it.assetId,it.manifestHash,GiantTileAddress(it.address.level,it.address.x,it.address.y),it.documentX,it.documentY,it.documentWidth,it.documentHeight,it.width,it.height,it.rgba)}}}
    override suspend fun close(){owner.close{try{handle.shutdown()}finally{handle.destroy()}}}
}
