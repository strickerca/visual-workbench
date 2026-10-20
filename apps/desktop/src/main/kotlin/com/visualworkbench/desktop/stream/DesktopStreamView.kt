package com.visualworkbench.desktop.stream

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.atomic.AtomicBoolean
import javax.swing.JPanel

internal data class DesktopStreamViewState(val signal:CaptureStreamSignal,
    val displayed:CaptureStreamFrame?=null,val lossless:Boolean=false,
    val sealed:Boolean=false,val failure:String?=null)
internal data class DesktopStreamContact(val frame:CaptureStreamFrame,val point:Point)

/** One local receiver canvas. The paired project/link and annotation draft are
 * borrowed, never replaced or closed here. Root must join stopLiveDraws() before
 * accepting pen DOWN and join close() before closing its stream/link. */
internal class DesktopStreamView(private val scope:CoroutineScope,private val core:WorkbenchCore,
    private val stream:WorkbenchCaptureStream,private val attached:()->Boolean,
    private val resources:SessionResources=sessionResources(),
    private val ui:CoroutineDispatcher=Dispatchers.Main.immediate,
    backendFactory:(WorkbenchCore,()->Unit,(StreamRaster)->Unit,(Throwable)->Unit)->DesktopStreamBackend=::DesktopStreamPainter) {
    private val initial=stream.signal()
    private val mutable=MutableStateFlow(DesktopStreamViewState(initial))
    val state:StateFlow<DesktopStreamViewState> = mutable.asStateFlow()
    private val wake=Channel<Unit>(Channel.CONFLATED)
    private val quarantinedRasters=mutableListOf<StreamRaster>()
    private val failedDraws=mutableListOf<CaptureStreamDraw>()
    private val failedBackgrounds=mutableListOf<CaptureFrozenBackground>()
    private val pending=AtomicReference<StreamCanvasSlot?>(null)
    private val surfaceGate=Mutex()
    private var surfaceLease:StreamSurfaceLease?=null
    private val adoptionGate=Mutex()
    private val closeGate=Mutex()
    @Volatile private var adoptionJob:Job?=null
    @Volatile private var admitting=true
    @Volatile private var closed=false
    private var closeFailure:Throwable?=null
    private val painter=backendFactory(core,{wake.trySend(Unit)},::quarantineRaster,::paintFailed)
    private val collector=scope.launch {
        try {stream.signals.collect { signal ->
            if(signal.connectionEpoch!=initial.connectionEpoch){seal("The capture connection changed. Close this view before opening another.");return@collect}
            mutable.value=mutable.value.copy(signal=signal)
            wake.trySend(Unit)
        }}catch(cancel:CancellationException){throw cancel}
        catch(_:Exception){seal("The capture signal stopped. The last verified frame remains visible.")}
    }
    private val worker=scope.launch {
        wake.trySend(Unit)
        try {for(ignored in wake){
            if(!admitting||closed)break
            val current=stream.signal()
            if(current.connectionEpoch!=initial.connectionEpoch){seal("The capture connection changed.");break}
            if(current.stage!=CaptureStreamStage.Live||!current.sourceAvailable)continue
            if(!withContext(ui){attached()}){seal("The capture view detached.");break}
            if(!withContext(ui){painter.canStartDraw()})continue
            val surface=ensureSurface()
            val draw=stream.beginDraw()?:continue
            try{process(draw,surface)}catch(cancel:CancellationException){throw cancel}
            catch(_:Exception){seal("The capture frame could not be presented. Close this view before retrying.");break}
        }}catch(cancel:CancellationException){throw cancel}
        catch(_:Exception){seal("The capture receiver stopped. The last verified frame remains visible.")}
    }

    fun container():JPanel=painter.container
    suspend fun setCamera(value:Camera?){withContext(ui){painter.setCamera(value)}}
    suspend fun mapPointer(value:Point):Point?=withContext(ui){painter.mapPointer(value)}
    private fun quarantineRaster(value:StreamRaster){synchronized(quarantinedRasters){quarantinedRasters+=value}}
    private fun paintFailed(@Suppress("UNUSED_PARAMETER") error:Throwable){seal("The capture canvas could not finish drawing. The last verified frame remains owned.")}
    private fun seal(message:String){admitting=false;mutable.value=mutable.value.copy(sealed=true,failure=message);pending.get()?.permit?.refuse()}
    private suspend fun ensureSurface():StreamSurfaceLease=surfaceGate.withLock {
        surfaceLease?.let{return@withLock it}
        val admitted=withContext(NonCancellable){reserveStreamSurface(resources)}
        surfaceLease=admitted
        currentCoroutineContext().ensureActive()
        admitted
    }

    private suspend fun process(draw:CaptureStreamDraw,surfaceLease:StreamSurfaceLease){
        var raster:StreamRaster?=null
        var slot:StreamCanvasSlot?=null
        var offered=false
        val rasterHandoff=AtomicReference<StreamRaster?>(null)
        val offerHandoff=AtomicBoolean(false)
        var problem:Throwable?=null
        try {
            val frame=draw.frame
            val current=stream.signal()
            if(admitting&&current.connectionEpoch==initial.connectionEpoch&&current.sourceGeneration==frame.sourceGeneration&&
                current.stage==CaptureStreamStage.Live&&current.sourceAvailable){
                val surface=withContext(ui){painter.surfaceBytes()}
                raster=withContext(Dispatchers.Default){
                    readStreamRaster(draw,resources,surfaceLease,surface,::quarantineRaster).also(rasterHandoff::set)
                }
                currentCoroutineContext().ensureActive()
                val after=stream.signal()
                if(admitting&&after.connectionEpoch==initial.connectionEpoch&&after.sourceGeneration==frame.sourceGeneration&&
                    after.stage==CaptureStreamStage.Live&&after.sourceAvailable&&withContext(ui){attached()}){
                    val candidate=StreamCanvasSlot(frame,checkNotNull(raster),StreamDrawPermit())
                    slot=candidate;pending.set(candidate)
                    offered=withContext(ui){painter.offer(candidate).also(offerHandoff::set)}
                    if(offered)candidate.permit?.awaitDecision()
                }
            }
        }catch(error:Throwable){problem=error}
        withContext(NonCancellable){
            raster=raster?:rasterHandoff.get()
            offered=offered||offerHandoff.get()
            val candidate=slot
            val receipt=if(candidate!=null){candidate.permit?.refuse();candidate.permit?.awaitDecision()}else false
            var acknowledged=false
            try{if(receipt!=null){draw.presented(receipt);acknowledged=receipt}}
            catch(error:Throwable){if(problem==null)problem=error else problem.addSuppressed(error)}
            try{draw.close()}catch(error:Throwable){synchronized(failedDraws){failedDraws+=draw};if(problem==null)problem=error else problem.addSuppressed(error)}
            try{
                if(candidate!=null){
                    if(acknowledged){
                        withContext(ui){painter.commit(candidate)}
                        mutable.value=mutable.value.copy(displayed=candidate.frame,lossless=false)
                    }else withContext(ui){painter.rollback(candidate,offered)}
                }else raster?.close()
            }catch(error:Throwable){
                if(candidate==null)raster?.let(::quarantineRaster)
                if(problem==null)problem=error else problem.addSuppressed(error)
            }finally{pending.compareAndSet(candidate,null)}
        }
        problem?.let{throw it}
    }

    /** Seal and settle every entered draw before the root calls firstContact.
     * The returned identity is the last positively acknowledged display. */
    suspend fun stopLiveDraws():CaptureStreamFrame? {
        admitting=false
        pending.get()?.permit?.refuse()
        worker.cancel()
        withContext(NonCancellable){worker.join()}
        val shown=mutable.value.displayed
        return shown?.takeUnless{mutable.value.sealed||
            (mutable.value.signal.sourceAvailable&&it.sourceGeneration!=mutable.value.signal.sourceGeneration)}
    }
    /** Invoke only for an accepted physical DOWN, after rejecting palms/tools. */
    suspend fun stopForContact(viewPoint:Point):DesktopStreamContact? {
        val frame=stopLiveDraws()?:return null
        val sourcePoint=mapPointer(viewPoint)?:return null
        return DesktopStreamContact(frame,sourcePoint)
    }

    /** A second exact frozen-background lease is copied into a separately
     * charged bitmap. The draft's preview asset and external camera never move. */
    suspend fun adoptLossless(session:CaptureAnnotationSession):Boolean =
        adoptLossless({session.refreshOriginal()},{session.state.value})
    internal suspend fun adoptLossless(refreshOriginal:suspend ()->Unit,
        snapshot:()->CaptureAnnotationState):Boolean=adoptionGate.withLock {
        currentCoroutineContext().ensureActive()
        adoptionJob=currentCoroutineContext()[Job]
        var raster:StreamRaster?=null
        val rasterHandoff=AtomicReference<StreamRaster?>(null)
        var slot:StreamCanvasSlot?=null
        var offered=false
        try {
            if(closed||mutable.value.sealed)return@withLock false
            check(!admitting&&withContext(ui){attached()})
            refreshOriginal()
            val owned=snapshot()
            val draft=owned.draft?:return@withLock false
            if(!owned.lossless||owned.frozen?.draftId!=draft.draftId)return@withLock false
            if(mutable.value.lossless&&mutable.value.displayed==draft.frame)return@withLock true
            val background=stream.frozenBackground()?:return@withLock false
            try {
                require(losslessBackgroundMatches(owned,background,initial.connectionEpoch,stream.signal()))
                val surfaceLease=ensureSurface()
                val surface=withContext(ui){painter.surfaceBytes()}
                raster=withContext(Dispatchers.Default){
                    readStreamRaster(background,resources,surfaceLease,surface,::quarantineRaster).also(rasterHandoff::set)
                }
            }finally{
                withContext(NonCancellable){try{background.close()}catch(error:Throwable){
                    synchronized(failedBackgrounds){failedBackgrounds+=background};throw error
                }}
            }
            currentCoroutineContext().ensureActive()
            if(!withContext(ui){adoptionValid(draft,snapshot)})return@withLock false
            val candidate=StreamCanvasSlot(draft.frame,checkNotNull(raster),losslessPaint=CompletableDeferred())
            slot=candidate
            offered=withContext(ui){painter.offer(candidate)}
            if(!offered)return@withLock false
            checkNotNull(candidate.losslessPaint).await()
            currentCoroutineContext().ensureActive()
            val committed=withContext(NonCancellable+ui){
                if(adoptionValid(draft,snapshot)){painter.commit(candidate);true}else false
            }
            if(!committed)return@withLock false
            slot=null;raster=null;rasterHandoff.set(null)
            mutable.value=mutable.value.copy(displayed=draft.frame,lossless=true)
            true
        }catch(cancel:CancellationException){throw cancel}
        catch(error:Throwable){seal("The lossless canvas could not finish adoption. Keep the paired project open until retirement.");throw error}
        finally{
            try{withContext(NonCancellable){
                raster=raster?:rasterHandoff.get()
                slot?.let{candidate->
                    val installed=try{withContext(ui){painter.isCommitted(candidate)}}catch(error:Throwable){
                        quarantineRaster(candidate.raster)
                        seal("The lossless canvas ownership could not be verified; keep the paired project open.")
                        throw error
                    }
                    if(installed){slot=null;raster=null;rasterHandoff.set(null)}
                    else try{withContext(ui){painter.rollback(candidate,offered)}}catch(error:Throwable){seal("The lossless canvas could not retire its provisional bitmap.");throw error}
                }
                if(slot==null)raster?.let{try{it.close()}catch(error:Throwable){quarantineRaster(it);seal("The lossless bitmap could not retire.");throw error}}
            }}finally{adoptionJob=null}
        }
    }

    private fun adoptionValid(draft:CaptureDraftInfo,snapshot:()->CaptureAnnotationState):Boolean {
        if(closed||mutable.value.sealed||stream.signal().connectionEpoch!=initial.connectionEpoch||!attached())return false
        val state=snapshot()
        return state.draft==draft&&state.lossless&&state.frozen?.draftId==draft.draftId
    }

    /** Does not close stream: CaptureAnnotationSession/root owns that lease. */
    suspend fun close(){
        val caller=currentCoroutineContext()[Job]
        withContext(NonCancellable){closeGate.withLock {
            closeFailure?.let{throw it}
            if(closed)return@withLock
            closed=true;admitting=false;pending.get()?.permit?.refuse()
            var failure:Throwable?=null
            try{worker.cancelAndJoin()}catch(error:Throwable){failure=error}
            try{collector.cancelAndJoin()}catch(error:Throwable){if(failure==null)failure=error else failure.addSuppressed(error)}
            adoptionJob?.takeIf{it!==caller}?.let{try{it.cancelAndJoin()}catch(error:Throwable){if(failure==null)failure=error else failure.addSuppressed(error)}}
            var canvasRetired=false
            adoptionGate.withLock {
                try{withContext(ui){painter.retire()};canvasRetired=true}catch(error:Throwable){if(failure==null)failure=error else failure.addSuppressed(error)}
            }
            if(canvasRetired)try{surfaceLease?.close();surfaceLease=null}catch(error:Throwable){
                if(failure==null)failure=error else failure.addSuppressed(error)
            }
            if(synchronized(quarantinedRasters){quarantinedRasters.isNotEmpty()}||
                synchronized(failedDraws){failedDraws.isNotEmpty()}||synchronized(failedBackgrounds){failedBackgrounds.isNotEmpty()})
                failure= failure?:IllegalStateException("Capture canvas retirement is incomplete")
            closeFailure=failure
            mutable.value=mutable.value.copy(sealed=true,failure=if(failure==null)null else "Capture canvas retirement is incomplete; keep the paired project open.")
            failure?.let{throw it}
        }}
    }
}
