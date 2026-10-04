package com.visualworkbench.shared

import com.visualworkbench.bindings.core.Cancellation
import com.visualworkbench.bindings.core.PackageException
import com.visualworkbench.bindings.core.McpResultInbox as NInbox
import com.visualworkbench.bindings.core.McpInboxSubmission as NSubmission
import com.visualworkbench.bindings.core.McpInboxReceipt as NReceipt
import com.visualworkbench.bindings.core.McpInboxImage as NImage
import com.visualworkbench.bindings.core.McpInboxSide as NSide
import com.visualworkbench.bindings.core.McpInboxRegion as NRegion
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore

private val inboxScope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
private val inboxSlots=Semaphore(2)
private fun failure(error:PackageException)=PackageFailure(when(error){
    is PackageException.Invalid->PackageFailureKind.Invalid;is PackageException.Limit->PackageFailureKind.Limit
    is PackageException.Stale->PackageFailureKind.Stale;is PackageException.Original->PackageFailureKind.Original
    is PackageException.Depth->PackageFailureKind.Depth;is PackageException.Unsupported->PackageFailureKind.Unsupported
    is PackageException.Integrity->PackageFailureKind.Integrity;is PackageException.Identity->PackageFailureKind.Identity
    is PackageException.Storage->PackageFailureKind.Storage;is PackageException.Busy->PackageFailureKind.Busy
    is PackageException.Closed->PackageFailureKind.Closed;is PackageException.Cancelled->PackageFailureKind.Cancelled})
private suspend fun<T> settled(release:suspend(T)->Unit={},block:suspend(Cancellation)->T):T=
    ownPackageResult(inboxScope,inboxSlots,::Cancellation,{it.cancel()},{it.destroy()},release){token->
        try{block(token)}catch(error:PackageException){throw failure(error)}catch(_:IllegalStateException){throw PackageFailure(PackageFailureKind.Closed)}}
private fun NImage.public()=McpInboxImage(width,height,bitDepth,encodedBytes,blake3,iccBlake3,orientationApplied)
private fun NReceipt.public()=McpInboxReceipt(receiptId,receiptBlake3,packageId,target,manifestSha256,workflowBinding(binding),sourceAssetId,connection,createdAtMs,text,note,before.public(),after?.public(),retired)
private fun McpInboxSide.native():NSide=when(this){McpInboxSide.Before->NSide.BEFORE;McpInboxSide.After->NSide.AFTER}
public actual suspend fun openMcpResultInbox(applicationPrivateDirectory:String):WorkbenchMcpInbox {
    if(applicationPrivateDirectory.length>32767)throw PackageFailure(PackageFailureKind.Invalid)
    prepareCoreLibrary()
    return settled(release={it.close()}){NativeInbox(com.visualworkbench.bindings.core.openMcpResultInbox(applicationPrivateDirectory,it))}
}
private class NativeInbox(private val handle:NInbox):WorkbenchMcpInbox {
    private val closer=JoinedPackageClose()
    override suspend fun submit(catalog:WorkbenchPackageCatalog,submission:McpInboxSubmission):McpInboxReceipt {
        closer.check();val value=ownedMcpSubmission(submission);val nativeCatalog=nativePackageCatalog(catalog)
        return settled{handle.submit(nativeCatalog,NSubmission(value.receiptId,value.packageId,value.target,value.manifestSha256,value.connection,value.createdAtMs,value.text,value.png,value.note),it).public()}
    }
    override suspend fun list():List<McpInboxReceipt>{closer.check();return settled{handle.list(it).map{r->r.public()}}}
    override suspend fun readPng(receiptId:String,receiptBlake3:String,side:McpInboxSide):ByteArray {
        closer.check();return settled{handle.readPng(receiptId,receiptBlake3,side.native(),it)}
    }
    override suspend fun pixels(receiptId:String,receiptBlake3:String,side:McpInboxSide,region:McpInboxRegion,assumeUntaggedSrgb:Boolean,allowDepthReduction:Boolean):McpInboxPixels {
        closer.check();if(region.width !in 1u..512u||region.height !in 1u..512u)throw PackageFailure(PackageFailureKind.Limit)
        return settled{handle.pixels(receiptId,receiptBlake3,side.native(),NRegion(region.x,region.y,region.width,region.height),assumeUntaggedSrgb,allowDepthReduction,it).let{p->
            if(p.receiptId!=receiptId||p.receiptBlake3!=receiptBlake3||p.side!=side.native()||p.region.x!=region.x||p.region.y!=region.y||p.region.width!=region.width||p.region.height!=region.height||p.rgbaSrgb.size.toULong()!=region.width.toULong()*region.height.toULong()*4uL)throw PackageFailure(PackageFailureKind.Integrity)
            McpInboxPixels(receiptId,receiptBlake3,side,region,p.canvasWidth,p.canvasHeight,p.rgbaSrgb)}}
    }
    override suspend fun retire(receiptId:String,receiptBlake3:String):McpInboxRetirement {
        closer.check();return settled{handle.retire(receiptId,receiptBlake3,it).let{r->McpInboxRetirement(r.receiptId,r.receiptBlake3,r.filesRemoved)}}
    }
    override suspend fun close(){closer.begin();releaseNative{closer.close{try{handle.shutdown()}finally{handle.destroy()}}}}
}
