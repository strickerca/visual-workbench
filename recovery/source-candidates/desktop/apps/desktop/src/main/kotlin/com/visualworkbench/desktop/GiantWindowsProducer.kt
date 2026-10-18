package com.visualworkbench.desktop

import com.visualworkbench.bindings.host.PyramidBuilder
import com.visualworkbench.bindings.host.PyramidOperation
import com.visualworkbench.bindings.host.PyramidRequest
import com.visualworkbench.bindings.host.PyramidReceipt
import com.visualworkbench.bindings.host.createPyramidBuilder
import com.visualworkbench.shared.GiantInputInfo
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withLock
import java.nio.file.Path
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

/** Data admission and shutdown use distinct bounded lanes. A saturated build
 * queue cannot prevent cancellation/retirement of its two admitted owners. */
private object PyramidCalls {
    val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
    val data=Semaphore(2)
    val controls=Semaphore(2)
    suspend fun <T> call(control:Boolean=false,release:suspend(T)->Unit={},block:suspend(PyramidOperation)->T):T {
        currentCoroutineContext().ensureActive()
        val slots=if(control)controls else data
        if(!slots.tryAcquire())throw HandoffFailure("The giant producer is busy. Retry after the current operation settles.")
        val operation=try{PyramidOperation()}catch(error:Throwable){slots.release();throw error}
        val handoff=AtomicReference<T?>(null)
        var producer:Deferred<T>?=null
        try {
            val work=scope.async{block(operation).also{handoff.set(it)}};producer=work
            try {val result=work.await();currentCoroutineContext().ensureActive();handoff.set(null);return result}
            catch(cancel:CancellationException){operation.cancel();throw cancel}
        }finally{
            withContext(NonCancellable){
                try {
                    producer?.let{if(!it.isCompleted){operation.cancel();try{it.await()}catch(_:Exception){}}}
                    handoff.getAndSet(null)?.let{release(it)}
                }finally{try{operation.destroy()}finally{slots.release()}}
            }
        }
    }
}

/** Holds approved-runtime pins through generated-future settlement. No codec is
 * loaded in the app process. Caller owns the destination directory until build
 * returns/cancels; a successful directory is then reverified by core publish. */
internal interface GiantProducer { suspend fun build(info:GiantInputInfo,destination:Path):PyramidReceipt; suspend fun close() }

internal class GiantWindowsProducer private constructor(private val native:PyramidBuilder,private val runtime:DesktopVipsRuntime) : GiantProducer {
    private val closing=AtomicBoolean(false)
    private val operationGate=Mutex()
    private val closeGate=Mutex()
    private var closed:Result<Unit>?=null
    companion object {
        suspend fun create(packaged:DesktopNativeRuntime):GiantWindowsProducer {
            val owned=AtomicReference<DesktopVipsRuntime?>(null)
            var native:PyramidBuilder?=null
            try{
                // Populate the handoff BEFORE crossing a cancellable dispatcher.
                val runtime=withContext(Dispatchers.IO){DesktopVipsRuntime.prepare(packaged.directory.parent.parent).also{owned.set(it)}}
                currentCoroutineContext().ensureActive()
                val handle=PyramidCalls.call(release={value:PyramidBuilder->try{value.shutdown()}finally{value.destroy()}}){op->
                    createPyramidBuilder(packaged.directory.resolve("vw-pyramid-helper.exe").toString(),packaged.packagedHash("vw-pyramid-helper.exe"),runtime.directory.toString(),op)
                };native=handle
                currentCoroutineContext().ensureActive()
                val result=GiantWindowsProducer(handle,runtime);native=null;owned.set(null);return result
            }finally{withContext(NonCancellable){try{native?.let{try{it.shutdown()}finally{it.destroy()}}}finally{owned.getAndSet(null)?.close()}}}
        }
    }
    override suspend fun build(info:GiantInputInfo,destination:Path):PyramidReceipt=operationGate.withLock {
        currentCoroutineContext().ensureActive();check(!closing.get())
        require(destination.isAbsolute)
        val receipt=PyramidCalls.call{op->native.build(PyramidRequest(info.sourcePath,destination.toString(),info.assetId,
            info.encodedBytes,info.width,info.height,info.orientation,16uL*1024uL*1024uL*1024uL,600_000uL),op)}
        if(receipt.directory!=destination.toString()||receipt.sourceAssetId!=info.assetId||receipt.width!=info.width||receipt.height!=info.height||
            receipt.tileCount==0u||receipt.tileCount>100_000u||receipt.encodedBytes==0uL||receipt.encodedBytes>16uL*1024uL*1024uL*1024uL||!Regex("[0-9a-f]{64}").matches(receipt.manifestHash))
            throw HandoffFailure("The giant producer receipt differs from the immutable source. Its derived output was not published as a project.")
        receipt
    }
    /** On failure, retain this owner and caller staging for explicit recovery;
     * never infer that an uncertain native worker has stopped using its files. */
    override suspend fun close(){closing.set(true);withContext(NonCancellable){closeGate.withLock{
        val result=closed?:runCatching{
            PyramidCalls.call(control=true){native.shutdown()}
            operationGate.withLock{native.destroy();runtime.close()}
        }.also{closed=it}
        result.getOrThrow()
    }}}
}
