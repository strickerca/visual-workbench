package com.visualworkbench.desktop

import com.visualworkbench.bindings.host.CaptureSelection
import com.visualworkbench.bindings.host.PickerMonitor
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

internal data class DesktopStreamPublicationState(val active:Boolean=false,val source:String?=null,
    val paused:Boolean=false,val capturedAtMs:Long?=null,val cleanupPending:Boolean=false,val issue:String?=null)
/** Source choice remains bound even when a reconnect reuses LiveAttachment or
 * a conflated status observer misses the intermediate disconnected state. */
internal class LiveCaptureConnectionFence(private val attached:LiveAttachment,private val currentEpoch:()->ULong){
    private val transition=attached.previewEpoch
    private val epoch=currentEpoch().also{require(it!=0uL)}
    fun current():Boolean=!attached.retiring&&attached.acceptsPreviews&&attached.previewEpoch==transition&&currentEpoch()==epoch
}
internal fun CaptureSelection.localSelection():CaptureLocalSelection=when(this){
    is CaptureSelection.Window->CaptureLocalSelection.Window(CaptureLocalWindow(target.window,target.processId,target.processCreated,
        CaptureLocalRect(target.clientX,target.clientY,target.clientWidth,target.clientHeight),
        CaptureLocalRect(target.frameX,target.frameY,target.frameWidth,target.frameHeight),target.dpi,target.observedNs))
    is CaptureSelection.Monitor->CaptureLocalSelection.Monitor(target.localMonitor())
    is CaptureSelection.Region->CaptureLocalSelection.Region(target.localMonitor(),CaptureLocalRect(x,y,width,height))
}
private fun PickerMonitor.localMonitor():CaptureLocalMonitor=CaptureLocalMonitor(monitor,device,CaptureLocalRect(x,y,width,height),dpi,rotation,primary,observedNs)
internal fun CaptureLocalSelection.publicationSource(id:String,name:String):CapturePublicationSource {
    val rect=when(this){is CaptureLocalSelection.Window->target.client;is CaptureLocalSelection.Monitor->target.physical;is CaptureLocalSelection.Region->region}
    val dpi=when(this){is CaptureLocalSelection.Window->target.dpi;is CaptureLocalSelection.Monitor->target.dpi;is CaptureLocalSelection.Region->target.dpi}
    val observed=when(this){is CaptureLocalSelection.Window->target.observedNs;is CaptureLocalSelection.Monitor->target.observedNs;is CaptureLocalSelection.Region->target.observedNs}
    return CapturePublicationSource(true,id,when(this){is CaptureLocalSelection.Window->"window";is CaptureLocalSelection.Monitor->"monitor";is CaptureLocalSelection.Region->"region"},
        (this as? CaptureLocalSelection.Window)?.target?.window?:0uL,when(this){is CaptureLocalSelection.Window->"";is CaptureLocalSelection.Monitor->target.device;is CaptureLocalSelection.Region->target.device},
        rect.x,rect.y,rect.width,rect.height,dpi.toDouble()/96.0,1u,observed.toLong(),15u,sourceName=name.take(128).ifBlank{"Selected source"})
}

/** A single retained acquisition/producer/publisher owner. Close seals first,
 * cancels and joins acquisition, then retires the exact child before its link. */
internal class DesktopStreamPublication(private val scope:CoroutineScope,private val report:(String)->Unit,
    private val openPublisher:suspend(ProjectLink,CapturePublicationSource)->WorkbenchCapturePublisher={link,source->link.openCapturePublisher(source,512uL*1024uL*1024uL)},
    private val openProducer:suspend(WorkbenchCapturePublisher,CaptureLocalOptions)->WorkbenchLocalCapture={publisher,options->publisher.openLocalCapture(options)}){
    private val gate=Mutex()
    private val mutable=MutableStateFlow(DesktopStreamPublicationState())
    val state=mutable.asStateFlow()
    private class Entry(val current:()->Boolean,val name:String){
        var closing=false;var acquisition:Deferred<Unit>?=null;var pump:Job?=null
        var publisher:WorkbenchCapturePublisher?=null;var producer:WorkbenchLocalCapture?=null
    }
    private var owner:Entry?=null
    suspend fun start(link:ProjectLink,source:CapturePublicationSource,options:CaptureLocalOptions,current:()->Boolean){
        currentCoroutineContext().ensureActive()
        close()
        val entry=Entry(current,source.sourceName)
        gate.withLock {
            check(owner==null)
            if(!current())throw SessionFailure(SessionFailureKind.Closed)
            owner=entry
            mutable.value=DesktopStreamPublicationState(active=true,source=entry.name)
            entry.acquisition=scope.async(start=CoroutineStart.LAZY){
                entry.publisher=openPublisher(link,source)
                requireCurrent(entry)
                entry.producer=openProducer(checkNotNull(entry.publisher),options)
                requireCurrent(entry)
                entry.pump=scope.launch {
                    try {
                        while(isActive){
                            requireCurrent(entry)
                            val publisher=checkNotNull(entry.publisher)
                            val signal=publisher.signal()
                            if(signal.closed)throw SessionFailure(SessionFailureKind.Closed)
                            checkNotNull(entry.producer).pumpOnce()
                            requireCurrent(entry)
                            mutable.value=mutable.value.copy(paused=signal.paused,capturedAtMs=signal.original?.frame?.capturedAtMs)
                            delay(if(signal.paused)100 else 16)
                        }
                    }catch(error:CancellationException){throw error}
                    catch(error:Exception){
                        if(owner===entry){
                            mutable.value=mutable.value.copy(issue=sessionMessage(error))
                            report("Live source stopped. ${sessionMessage(error)}")
                            // A separate cleanup coroutine can join this pump.
                            scope.launch {try{retire(entry)}catch(_:Exception){}}
                        }
                    }
                }
            }
        }
        try {checkNotNull(entry.acquisition).await()}
        catch(error:Throwable){withContext(NonCancellable){try{retire(entry)}catch(cleanup:Throwable){if(cleanup!==error)error.addSuppressed(cleanup)}};throw error}
    }
    private fun requireCurrent(entry:Entry){if(owner!==entry||entry.closing||!entry.current())throw SessionFailure(SessionFailureKind.Closed)}
    suspend fun close(){owner?.let{retire(it)}}
    private suspend fun retire(entry:Entry):Unit=withContext(NonCancellable){
        // Seal before waiting for the serialized close lock.
        entry.closing=true
        entry.acquisition?.cancel()
        gate.withLock {
            if(owner!==entry)return@withLock
            try {
                entry.producer?.cancel()
                entry.acquisition?.cancelAndJoin();entry.acquisition=null
                entry.pump?.cancelAndJoin();entry.pump=null
                entry.producer?.close();entry.producer=null
                entry.publisher?.close();entry.publisher=null
                owner=null
                mutable.value=DesktopStreamPublicationState(issue=mutable.value.issue)
            }catch(error:Throwable){mutable.value=mutable.value.copy(cleanupPending=true,issue="Live source cleanup is pending. Retry before leaving this project.");throw error}
        }
    }
}
