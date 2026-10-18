package com.visualworkbench.shared

import com.visualworkbench.bindings.core.Cancellation
import com.visualworkbench.bindings.core.GiantImageException
import com.visualworkbench.bindings.core.GiantImage as NImage
import com.visualworkbench.bindings.core.GiantOpenOptions
import com.visualworkbench.bindings.core.GiantTileAddress as NAddress
import com.visualworkbench.bindings.core.GiantViewport as NViewport
import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.SessionResourceCharge as NCharge
import com.visualworkbench.bindings.core.WorkflowBinding as NBinding
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

private val resourcePool by lazy { com.visualworkbench.bindings.core.sharedSessionResources() }
public actual fun sessionResources(): SessionResources {
    prepareCoreLibrary()
    return object : SessionResources {
        override suspend fun reserveForeign(bytes: ULong): SessionResourceCharge {
            currentCoroutineContext().ensureActive()
            val charge=giantNative { resourcePool.reserveForeign(bytes) }
            try {
                currentCoroutineContext().ensureActive()
                return NativeResourceCharge(charge, bytes)
            } catch(error:Throwable) { retireNativeDelivery(charge);throw error }
        }
    }
}
// Unknown release failures keep the exact opaque owner alive. A final wrapper
// drop must not silently return quota while its native lease remains uncertain.
private val failedDeliveryCharges=mutableListOf<NCharge>()
private fun retireNativeDelivery(charge:NCharge) {
    try {giantNative{charge.releaseCharge()};charge.destroy()}
    catch(error:Throwable){synchronized(failedDeliveryCharges){failedDeliveryCharges+=charge};throw error}
}
private class NativeResourceCharge(private val native: NCharge,private val bytes:ULong):SessionResourceCharge {
    @Synchronized fun requireLive(minimum:ULong) {
        if(closed||bytes<minimum)throw GiantImageFailure(GiantImageFailureKind.Memory)
    }
    private var closed=false
    @Synchronized override fun close() {
        if(closed)return
        giantNative{native.releaseCharge()};closed=true;native.destroy()
    }
}
private inline fun <T> giantNative(block:()->T):T = try{block()}catch(error:GiantImageException){throw mapGiantFailure(error)}
internal fun mapGiantFailure(error:GiantImageException):GiantImageFailure {
    val kind=when(error){
        is GiantImageException.Invalid->GiantImageFailureKind.Invalid
        is GiantImageException.Limit->GiantImageFailureKind.Limit
        is GiantImageException.Stale->GiantImageFailureKind.Stale
        is GiantImageException.Missing->GiantImageFailureKind.Missing
        is GiantImageException.Unsupported->GiantImageFailureKind.Unsupported
        is GiantImageException.Closed->GiantImageFailureKind.Closed
        is GiantImageException.Cancelled->GiantImageFailureKind.Cancelled
        is GiantImageException.Backpressure->GiantImageFailureKind.Backpressure
        is GiantImageException.Storage->GiantImageFailureKind.Storage
        is GiantImageException.Memory->GiantImageFailureKind.Memory
    }
    return if(error is GiantImageException.Memory)GiantImageFailure(kind,error.estimated,error.budget,error)
        else GiantImageFailure(kind,cause=error)
}
private val giantRetirement=Mutex()
private val unadoptedGiants=mutableListOf<GiantImage>()
internal suspend fun disposeUnadoptedGiantImage(image:GiantImage):Unit=withContext(NonCancellable){
    giantRetirement.withLock {
        try{image.close()}catch(error:Throwable){unadoptedGiants+=image;throw error}
    }
}
internal suspend fun drainUnadoptedGiants():Unit=withContext(NonCancellable){giantRetirement.withLock{
    val iterator=unadoptedGiants.iterator()
    while(iterator.hasNext()){iterator.next().close();iterator.remove()}
}}
internal suspend fun openGiantImageNative(project: ProjectSession,binding:WorkflowBinding,
    diskCacheBytes:ULong,ramCacheBytes:ULong):GiantImage {
    drainUnadoptedGiants()
    val options=GiantOpenOptions(NBinding(binding.projectId,binding.documentId,binding.hostSeq,binding.stateHash),diskCacheBytes,ramCacheBytes)
    val native=try{settledStream(release={value:NImage->disposeUnadoptedGiantImage(NativeGiantImage(value))}){project.openGiantImage(options,it)}}
        catch(error:GiantImageException){throw mapGiantFailure(error)}
    return NativeGiantImage(native)
}
private class NativeGiantImage(private val native:NImage):GiantImage {
    private val gate=Mutex();private var closed=false
    private suspend fun <T> call(block:suspend(Cancellation)->T):T=gate.withLock {
        if(closed)throw GiantImageFailure(GiantImageFailureKind.Closed)
        try{settledStream(block=block)}catch(error:GiantImageException){throw mapGiantFailure(error)}
    }
    override suspend fun viewport(view:GiantViewport):GiantViewportPlan = call {cancel->
        val value=native.viewport(NViewport(view.left,view.top,view.right,view.bottom,view.a,view.b,view.c,view.d),cancel)
        GiantViewportPlan(value.assetId,value.manifestHash,value.level,value.visible.map{GiantTileAddress(it.level,it.x,it.y)},value.prefetch.map{GiantTileAddress(it.level,it.x,it.y)})
    }
    override suspend fun tile(address:GiantTileAddress,foreign:SessionResourceCharge):GiantTilePixels=gate.withLock{
        if(closed)throw GiantImageFailure(GiantImageFailureKind.Closed)
        val reservation=foreign as? NativeResourceCharge ?: throw GiantImageFailure(GiantImageFailureKind.Invalid)
        reservation.requireLive(GiantViewportSession.TILE_CHARGE.toULong())
        // The private native delivery charge survives serialization and RustBuffer
        // decoding via this opaque lease. The caller's global foreign reservation
        // was admitted before this call and outlives actual renderer retirement.
        val delivery=try{settledStream(release={value:com.visualworkbench.bindings.core.GiantTileDelivery->
            retireNativeDelivery(value.nativeLease)
        }){native.tile(NAddress(address.level,address.x,address.y),it)}}
            catch(error:GiantImageException){throw mapGiantFailure(error)}
        try {
            currentCoroutineContext().ensureActive();reservation.requireLive(GiantViewportSession.TILE_CHARGE.toULong())
            val value=delivery.pixels
            GiantTilePixels(value.assetId,value.manifestHash,GiantTileAddress(value.address.level,value.address.x,value.address.y),
                value.documentX,value.documentY,value.documentWidth,value.documentHeight,value.width,value.height,value.rgba)
        } finally {retireNativeDelivery(delivery.nativeLease)}
    }
    override suspend fun close():Unit=withContext(NonCancellable){gate.withLock{
        if(!closed){giantNative{native.closeSession()};closed=true;native.destroy()}
    }}
}
