package com.visualworkbench.shared
import com.visualworkbench.bindings.core.Cancellation
import com.visualworkbench.bindings.core.PackageException
import com.visualworkbench.bindings.core.PackageCatalog as NCatalog
import com.visualworkbench.bindings.core.CompiledPackage as NCompiled
import com.visualworkbench.bindings.core.PackageTarget as NTarget
import com.visualworkbench.bindings.core.PackageCompileOptions as NOptions
import com.visualworkbench.bindings.core.PackageInfo as NInfo
import com.visualworkbench.bindings.core.PublishedPackage as NPublished
import com.visualworkbench.bindings.core.PackageRetirement as NRetirement
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
private val packageScope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
private val packageSlots=Semaphore(4)
private fun failure(error:PackageException)=PackageFailure(when(error){
    is PackageException.Invalid->PackageFailureKind.Invalid;is PackageException.Limit->PackageFailureKind.Limit
    is PackageException.Stale->PackageFailureKind.Stale;is PackageException.Original->PackageFailureKind.Original
    is PackageException.Depth->PackageFailureKind.Depth;is PackageException.Unsupported->PackageFailureKind.Unsupported
    is PackageException.Integrity->PackageFailureKind.Integrity;is PackageException.Identity->PackageFailureKind.Identity
    is PackageException.Storage->PackageFailureKind.Storage;is PackageException.Busy->PackageFailureKind.Busy
    is PackageException.Closed->PackageFailureKind.Closed;is PackageException.Cancelled->PackageFailureKind.Cancelled})
private suspend fun <T> settled(release:suspend(T)->Unit={},block:suspend(Cancellation)->T):T=
    ownPackageResult(packageScope,packageSlots,::Cancellation,{it.cancel()},{it.destroy()},release){token->
        try{block(token)}catch(error:PackageException){throw failure(error)}catch(error:IllegalStateException){throw PackageFailure(PackageFailureKind.Closed)}}
private fun NInfo.public()=PackageInfo(packageId,target,model,workflowBinding(binding),sourceAssetId,manifestSha256,createdAt,totalBytes,markerCount,images.map{PackageImageInfo(it.id,it.role,it.path,it.width,it.height,it.encodedBytes)},inlineImagesAvailable,includesWindowTitle)
private fun NPublished.public()=PublishedPackage(info.public(),directory)
private fun NRetirement.public()=PackageRetirement(packageId,target,manifestSha256,filesRemoved)
private fun PackageTarget.native():NTarget=when(this){
    is PackageTarget.ClaudeModern->NTarget.ClaudeModern(model);is PackageTarget.ClaudeLegacy->NTarget.ClaudeLegacy(model)
    is PackageTarget.OpenAiResponses->NTarget.OpenAiResponses(model,maxLongEdge)
    is PackageTarget.Gemini->NTarget.Gemini(model,maxLongEdge);is PackageTarget.Generic->NTarget.Generic(maxLongEdge)}
public actual suspend fun WorkbenchProject.compilePackage(options:PackageCompileOptions):WorkbenchCompiledPackage {
    val target=options.target
    val model=when(target){is PackageTarget.ClaudeModern->target.model;is PackageTarget.ClaudeLegacy->target.model;is PackageTarget.OpenAiResponses->target.model;is PackageTarget.Gemini->target.model;is PackageTarget.Generic->null}
    if(options.packageId.length!=36||model?.length?.let{it>256}==true||options.semanticSnapshotId?.length?.let{it!=36}==true)throw PackageFailure(PackageFailureKind.Invalid)
    return settled(release={it.close()}){cancel->
        val handle=nativeProjectHandle(this).compilePackage(NOptions(workflowBinding(options.binding),options.packageId,options.createdAtMs,target.native(),options.semanticSnapshotId,options.includeWindowTitle,options.assumeUntaggedSrgb,options.allowDepthReduction,options.memoryBudgetBytes),cancel)
        try{NativeCompiled(handle)}catch(error:Throwable){try{handle.dispose()}finally{handle.destroy()};throw error}
    }
}
private class NativeCompiled(val handle:NCompiled):WorkbenchCompiledPackage {
    private val closer=JoinedPackageClose()
    override val info:PackageInfo=handle.describe().public()
    fun check(){closer.check()}
    override suspend fun close(){closer.begin();releaseNative{closer.close{try{handle.dispose()}finally{handle.destroy()}}}}
}
public actual suspend fun openPackageCatalog(applicationPrivateDirectory:String):WorkbenchPackageCatalog {
    if(applicationPrivateDirectory.length>32767)throw PackageFailure(PackageFailureKind.Invalid)
    prepareCoreLibrary()
    return settled(release={it.close()}){NativeCatalog(com.visualworkbench.bindings.core.openPackageCatalog(applicationPrivateDirectory,it))}
}
private class NativeCatalog(val handle:NCatalog):WorkbenchPackageCatalog {
    private val closer=JoinedPackageClose()
    fun check(){closer.check()}
    override suspend fun publish(packageValue:WorkbenchCompiledPackage):PublishedPackage {
        check();val compiled=packageValue as? NativeCompiled?:throw PackageFailure(PackageFailureKind.Invalid);compiled.check()
        return settled{handle.publish(compiled.handle,it).public()}
    }
    override suspend fun list():List<PublishedPackage>{check();return settled{handle.list(it).map{p->p.public()}}}
    override suspend fun lookup(packageId:String,target:String,manifestSha256:String):PublishedPackage{check();return settled{handle.lookup(packageId,target,manifestSha256,it).public()}}
    override suspend fun readFile(packageId:String,target:String,manifestSha256:String,name:String,maxBytes:UInt):ByteArray{check();return settled{handle.readFile(packageId,target,manifestSha256,name,maxBytes,it)}}
    override suspend fun pendingRetirement():PackageRetirement?{check();return settled{handle.pendingRetirement(it)?.public()}}
    override suspend fun retire(packageId:String,target:String,manifestSha256:String):PackageRetirement{check();return settled{handle.retire(packageId,target,manifestSha256,it).public()}}
    override suspend fun close(){closer.begin();releaseNative{closer.close{try{handle.shutdown()}finally{handle.destroy()}}}}
}

/** Borrowed for a settled inbox operation; the catalog remains caller-owned. */
internal fun nativePackageCatalog(value:WorkbenchPackageCatalog):NCatalog {
    val catalog=value as? NativeCatalog?:throw PackageFailure(PackageFailureKind.Invalid)
    catalog.check();return catalog.handle
}
