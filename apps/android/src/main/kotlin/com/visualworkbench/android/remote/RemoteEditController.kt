package com.visualworkbench.android.remote

import android.os.SystemClock
import android.os.Build
import androidx.annotation.RequiresApi
import android.view.MotionEvent
import android.view.Surface
import com.visualworkbench.shared.*
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlin.math.roundToInt

internal data class RemoteRenderMapping(val ownerId:String,val frame:RemoteVideoFrame,val viewport:RemoteViewport,val rect:RemotePhysicalRect)
/** Read-only observation published only after the production current-owner
 * callback and retained native ticket/derived-PTS validation adopt a mapping.
 * A consumer cannot write it or create input/renderer authority from it. */
internal data class RemoteRenderedFrameObservation(val ownerId:String,val scope:RemoteVideoScope,
    val generation:ULong,val ticket:ULong,val frameId:ULong,val ptsUs:Long,
    val renderedNanos:Long,val callbackNanos:Long,val admission:DecoderAdmission,
    val acknowledgment:RemoteRenderedAcknowledgment?)
internal data class RemotePhoneDisplay(val config:RemoteVideoConfig?=null,val message:String?=null,val decoder:DecoderStatus?=null,val viewWidth:Int=0,val viewHeight:Int=0,val renderIdentity:DecoderRenderIdentity?=null)
internal data class RemoteControllerInputEvidence(val input:RemoteControllerInputObservation?,
    val rendered:RemoteRenderedFrameObservation?,val echo:RemoteEchoTiming,val ghosts:List<VisibleGhost>)
internal data class RemoteControllerInputObservation(val admission:RemoteInputAdmission,
    val phase:RemotePenPhase,val ghostAccepted:Boolean,val ghostSegment:GhostSegment?)
internal data class TouchAdmission(val binding:RemoteTargetBinding,val sample:RemotePenSample,val point:GhostPoint,val contact:Boolean,val fence:Long)
/** Owns every native ticket, duplicated Surface/codec worker, palette and input
 * queue until actual retirement. No callback or predicted sample invents ACK. */
internal class RemoteEditController(val remote:WorkbenchRemoteEdit,parent:CoroutineScope):RemoteConsumerOwner {
    private val job=SupervisorJob(parent.coroutineContext[Job])
    private val scope=CoroutineScope(parent.coroutineContext.minusKey(Job)+job)
    private val decoderMutex=Mutex()
    private val displayMutable=MutableStateFlow(RemotePhoneDisplay())
    val display:StateFlow<RemotePhoneDisplay> = displayMutable.asStateFlow()
    private val renderedMutable=MutableStateFlow<RemoteRenderedFrameObservation?>(null)
    val renderedObservation:StateFlow<RemoteRenderedFrameObservation?> = renderedMutable.asStateFlow()
    val actualRetired:Boolean get()=retired
    private val inputObservationMutable=MutableStateFlow<RemoteControllerInputObservation?>(null)
    val inputObservation:StateFlow<RemoteControllerInputObservation?> = inputObservationMutable.asStateFlow()
    val ghost=RemoteGhostInk()
    fun inputEvidence():RemoteControllerInputEvidence = synchronized(inputFence){
        RemoteControllerInputEvidence(inputObservationMutable.value,renderedMutable.value,echoMutable.value,ghost.snapshot(System.nanoTime()))
    }
    val inputClockDescription:String=if(Build.VERSION.SDK_INT>=34)"Nanosecond event fields; uptime alignment bounded to 1 ms" else "Millisecond event precision"
    val palette=RemoteEditorPalette(remote,scope)
    private val segments=RemoteGhostSegments(ghost)
    private val echoLedger=RemoteEchoLedger()
    private val echoMutable=MutableStateFlow(RemoteEchoTiming(null,0uL,0uL,0,null,null,0uL,0uL))
    val echo:StateFlow<RemoteEchoTiming> = echoMutable.asStateFlow()
    private val mapping=AtomicReference<RemoteRenderMapping?>(null)
    private val nativeTickets=linkedMapOf<ULong,RemoteVideoFrame>()
    private val input=Channel<TouchAdmission>(8)
    @Volatile private var borrowedSurface:Surface?=null
    @Volatile private var surfaceOwner:Any?=null
    private var decoder:RemoteHardwareDecoder?=null
    private var decoderConfig:RemoteVideoConfig?=null
    private var decoderSurfaceEpoch:Long?=null
    private val streamKeys=listOf("frames_received","scope_discarded","inactive_discarded","surface_waited","surface_deadline","surface_invalidated","decoder_opened","decoder_refused","startup_failed","frames_queued","queue_refused","rendered")
    private val streamCounts=Array(streamKeys.size){java.util.concurrent.atomic.AtomicLong()}
    private fun count(key:String){streamCounts[streamKeys.indexOf(key)].updateAndGet{if(it==Long.MAX_VALUE)it else it+1}}
    fun streamDiagnostics():Map<String,ULong> = streamKeys.indices.associate{streamKeys[it] to streamCounts[it].get().toULong()}
    private var statusJob:Job?=null
    private val inputFence=Any()
    private var admissionFence=0L
    private var surfaceEpoch=0L
    @Volatile private var foregroundActive=true
    @Volatile private var enabled=false
    @Volatile private var retired=false
    private var previousActual:Pair<RemoteTargetBinding,GhostPoint>?=null
    private var suppressedGesture=false
    private val inputGrantFence=RemoteInputGrantFence()
    init {
        remote.registerConsumer(this)
        // NativeRemoteEdit synchronously invalidates registered consumers before
        // publishing a replacement/retired binding. An asynchronous state watcher
        // here could cancel samples already admitted by the newer render/grant.
        scope.launch(Dispatchers.Default) {
            try {remote.frames.collect{frame->receive(frame)}}
            catch(cancelled:CancellationException){throw cancelled}
            catch(_:Exception){recoverInput();displayMutable.update{it.copy(message="Remote video stopped. Select the window and grant again on the computer.")}}
        }
        scope.launch(Dispatchers.Default) {
            for(item in input){
                val age=System.nanoTime()-item.sample.sampledLocalNanos
                if(age !in 0 until 50_000_000L){deactivate("input_expired");continue}
                val allowed=synchronized(inputFence){enabled&&foregroundActive&&item.fence==admissionFence&&remote.target.value?.let{it.grantActive&&it.binding==item.binding}==true}
                if(!allowed)continue
                try{
                    val admission=remote.pen(item.binding,item.sample)
                    synchronized(inputFence){
                        if(enabled&&foregroundActive&&item.fence==admissionFence&&remote.target.value?.let{it.grantActive&&it.binding==item.binding}==true){
                            echoLedger.record(admission);echoMutable.value=echoLedger.snapshot(System.nanoTime())
                            val before=previousActual?.takeIf{it.first==item.binding}?.second
                            val ghostAccepted=segments.admitted(admission,item.point,item.contact)
                            val admittedGhost=ghost.snapshot(System.nanoTime()).firstOrNull{it.segment.binding==admission.binding&&admission.sequence in it.segment.firstInputSequence..it.segment.lastInputSequence}?.segment
                            // Actual native transport admission only; predictions and
                            // queued MotionEvents never publish a sequence here.
                            inputObservationMutable.value=RemoteControllerInputObservation(admission,item.sample.phase,ghostAccepted,admittedGhost)
                            if(ghostAccepted&&item.contact&&before!=null){
                                val dt=item.point.localNanos-before.localNanos
                                if(dt in 1..50_000_000L){val horizon=12_000_000L;val factor=horizon.toFloat()/dt
                                    val dx=((item.point.x-before.x)*factor).coerceIn(-64f,64f)
                                    val dy=((item.point.y-before.y)*factor).coerceIn(-64f,64f)
                                    segments.prediction(item.point.copy(x=item.point.x+dx,y=item.point.y+dy,localNanos=item.point.localNanos+horizon,predicted=true))
                                }
                            }
                            previousActual=if(item.contact)item.binding to item.point else null
                        }
                    }
                }catch(cancelled:CancellationException){throw cancelled}
                catch(_:Exception){deactivate("owner_pause")}
            }
        }
    }
    // All ghost/input mutable state uses this short local fence. The synchronous
    // native queue reservation is state-only: no provider, IO or suspend work.
    private fun clearInput(){enabled=false;admissionFence++;inputObservationMutable.value=null;previousActual=null;suppressedGesture=false;segments.retire();echoLedger.retire();echoMutable.value=echoLedger.snapshot(System.nanoTime());while(input.tryReceive().isSuccess){}}
    fun resumeInput(){foreground()}
    fun foreground(){if(!retired){foregroundActive=true;scope.launch{runCatching{remote.requestKeyframe()}}}}
    fun deactivate(reason:String){if(reason=="background")foregroundActive=false;invalidate();remote.seal(reason)}
    // NativeRemoteEdit.seal calls consumer invalidation before native pause. This
    // callback stays local-only; a same-binding render cannot reopen its fence.
    private fun invalidateLocal():Boolean = synchronized(inputFence){
        val target=remote.target.value;val hadGrant=enabled||target?.grantActive==true
        inputGrantFence.invalidate(target?.binding);clearInput();mapping.set(null);renderedMutable.value=null
        hadGrant
    }
    override fun invalidate(){invalidateLocal()}
    private fun recoverInput(){
        // Capture grant and cancel local work in one fence. Never call seal under
        // inputFence: seal invokes this consumer callback. Input-free startup
        // recovery keeps viewing; canceled active input must retire natively.
        if(invalidateLocal())remote.seal("decoder_failure")
    }
    fun viewport(width:Int,height:Int){
        if(width!=display.value.viewWidth||height!=display.value.viewHeight){
            if(invalidateLocal())remote.seal("surface_lost")
        }
        displayMutable.update{it.copy(viewWidth=width,viewHeight=height)}
    }
    fun surface(value:Surface?,owner:Any){
        if(borrowedSurface===value)return
        val hadSurface=borrowedSurface!=null
        val activeGrant=invalidateLocal()
        val changedEpoch=synchronized(inputFence){surfaceEpoch++;borrowedSurface=value;surfaceOwner=owner;surfaceEpoch}
        if(hadSurface||value==null||activeGrant)remote.seal("surface_lost")
        scope.launch{
            decoderMutex.withLock{
                if(changedEpoch!=synchronized(inputFence){surfaceEpoch})return@launch
                // A first-IDR waiter may already have opened on this new Surface.
                // This attachment callback retires only a previous Surface owner.
                if(decoder!=null&&decoderSurfaceEpoch!=changedEpoch&&!closeDecoder())return@launch
            }
            if(value!=null&&!retired&&foregroundActive)runCatching{remote.requestKeyframe()}
        }
    }
    private suspend fun closeDecoder():Boolean {
        recoverInput()
        val current=decoder?:return true
        if(current.close() is DecoderRetirement.Pending){displayMutable.update{it.copy(message="Video retirement is pending; this owner remains held.")};return false}
        statusJob?.cancelAndJoin();statusJob=null;decoder=null;decoderConfig=null;decoderSurfaceEpoch=null
        for(ticket in nativeTickets.keys.toList())remote.discardFrame(ticket)
        nativeTickets.clear();return true
    }
    private suspend fun receive(frame:RemoteVideoFrame){decoderMutex.withLock{
        count("frames_received")
        val state=remote.state.value
        if(retired||!foregroundActive){count("inactive_discarded");remote.discardFrame(frame.ticket);return@withLock}
        if(frame.config.scope!=state.scope){count("scope_discarded");remote.discardFrame(frame.ticket);return@withLock}
        val startupDeadline=System.nanoTime()+START_DEADLINE_MILLIS*1_000_000L
        val initialSurfaceEpoch=synchronized(inputFence){surfaceEpoch}
        if(borrowedSurface?.isValid!=true){
            if(!frame.keyframe||borrowedSurface!=null){count("surface_invalidated");remote.discardFrame(frame.ticket);return@withLock}
            count("surface_waited")
            val ready=awaitRemoteSurfaceStartup(startupDeadline,
                isCurrent={!retired&&foregroundActive&&frame.config.scope==remote.state.value.scope&&synchronized(inputFence){surfaceEpoch in initialSurfaceEpoch..initialSurfaceEpoch+1}},
                isReady={borrowedSurface?.isValid==true&&surfaceOwner!=null})
            if(ready!=RemoteDecoderStartup.Ready){
                count(if(ready==RemoteDecoderStartup.Deadline)"surface_deadline" else "surface_invalidated")
                remote.discardFrame(frame.ticket)
                if(!retired&&foregroundActive&&frame.config.scope==remote.state.value.scope)runCatching{remote.requestKeyframe()}
                return@withLock
            }
        }
        displayMutable.update{it.copy(config=frame.config)}
        val current=decoder
        val configChanged=decoderConfig?.let{!sameConfig(it,frame.config)}==true
        val recover=current?.state?.value is DecoderStatus.RecoveryRequired
        if(configChanged||recover){recoverInput();if(!closeDecoder()){remote.discardFrame(frame.ticket);return@withLock}}
        // Retain this exact first IDR through asynchronous codec startup. The
        // decoder's worker starts separately; queue() deliberately refuses Starting.
        val admissionSurfaceEpoch=synchronized(inputFence){surfaceEpoch}
        if(decoder==null){
            if(!frame.keyframe){remote.discardFrame(frame.ticket);remote.requestKeyframe();return@withLock}
            val surface=borrowedSurface?.takeIf{it.isValid};val consumer=surfaceOwner
            if(surface==null||consumer==null){remote.discardFrame(frame.ticket);return@withLock}
            val decoderScope=frame.config.scope.decoder()
            val configuration=DecoderConfig(decoderScope,frame.config.generation,frame.config.codedWidth,frame.config.codedHeight,frame.config.visibleWidth,frame.config.visibleHeight,frame.config.vps,frame.config.sps,frame.config.pps)
            val ownerReference=AtomicReference<String?>(null)
            val expectedSurfaceEpoch=synchronized(inputFence){surfaceEpoch}
            when(val result=RemoteHardwareDecoder.open(surface,consumer,configuration){ticket,ptsUs,renderedNanos,callbackNanos->
                val expected=ownerReference.get()?:return@open
                scope.launch(Dispatchers.Default){rendered(expected,expectedSurfaceEpoch,ticket,ptsUs,renderedNanos,callbackNanos)}
            }) {
                is DecoderOpenResult.Refused->{count("decoder_refused");remote.discardFrame(frame.ticket);displayMutable.update{it.copy(message="Hardware HEVC refused: ${result.reason}")};runCatching{remote.pause("decoder_failure")};return@withLock}
                is DecoderOpenResult.Started->{count("decoder_opened");decoder=result.decoder;decoderConfig=frame.config;decoderSurfaceEpoch=expectedSurfaceEpoch;ownerReference.set(result.decoder.ownerId);statusJob=scope.launch{result.decoder.state.collect{status->displayMutable.update{it.copy(decoder=status,renderIdentity=result.decoder.lastRenderIdentity.value)};if(status is DecoderStatus.RecoveryRequired){
                    recoverInput()
                    // Backpressure waits for actual ticket settlement. A missing
                    // callback must therefore retire independently of next receive.
                    // This sibling job may join statusJob; the collector never joins itself.
                    scope.launch { retireRemoteDecoderRecovery(
                        retire={decoderMutex.withLock {
                            if(decoder!==result.decoder) RemoteRecoveryRetirement.Obsolete
                            else if(closeDecoder()) RemoteRecoveryRetirement.Complete
                            else RemoteRecoveryRetirement.Pending
                        }},
                        request={if(!retired&&foregroundActive&&remote.state.value.scope==frame.config.scope)runCatching{remote.requestKeyframe()}}
                    ) }
                }}}}
            }
        }
        val active=decoder?:return@withLock
        val admission=awaitRemoteDecoderStartup(active.state,startupDeadline){
            !retired&&foregroundActive&&borrowedSurface?.isValid==true&&decoder===active&&
                frame.config.scope==remote.state.value.scope&&sameConfig(requireNotNull(decoderConfig),frame.config)&&
                admissionSurfaceEpoch==synchronized(inputFence){surfaceEpoch}
        }
        if(admission!=RemoteDecoderStartup.Ready){
            count("startup_failed")
            // Startup never consumes/discards the only IDR before Ready. An
            // invalidated/cancelled owner cannot queue into a later scope/Surface.
            active.requestRecovery(if(admission==RemoteDecoderStartup.Deadline)"ControllerStartupDeadline" else "ControllerStartupInvalidated")
            remote.discardFrame(frame.ticket);recoverInput();closeDecoder()
            if(!retired&&foregroundActive&&frame.config.scope==remote.state.value.scope)runCatching{remote.requestKeyframe()}
            return@withLock
        }
        if(nativeTickets.size>=1){
            // Dropping any inter-frame AU invalidates its reference chain.
            active.requestRecovery("ControllerFrameLoss");recoverInput()
            remote.discardFrame(frame.ticket);closeDecoder();runCatching{remote.requestKeyframe()};return@withLock
        }
        nativeTickets[frame.ticket]=frame
        val queued=active.queue(DecoderFrame(frame.ticket,frame.config.scope.decoder(),frame.config.generation,frame.frameId,frame.ptsUs,frame.bytes,System.nanoTime(),frame.keyframe))
        if(queued !is DecoderQueueResult.Accepted){count("queue_refused");nativeTickets.remove(frame.ticket);remote.discardFrame(frame.ticket);recoverInput();runCatching{remote.requestKeyframe()}}else count("frames_queued")
    }}
    private suspend fun rendered(ownerId:String,expectedSurfaceEpoch:Long,ticket:ULong,ptsUs:Long,renderedNanos:Long,callbackNanos:Long){decoderMutex.withLock{
        val active=decoder?:return@withLock;val frame=nativeTickets[ticket]
        // A retired owner cannot poison a new decoder or consume its ticket.
        if(active.ownerId!=ownerId)return@withLock
        if(active.state.value is DecoderStatus.RecoveryRequired)return@withLock
        if(expectedSurfaceEpoch!=synchronized(inputFence){surfaceEpoch}||!foregroundActive||borrowedSurface?.isValid!=true)return@withLock
        if(frame==null||frame.ptsUs!=ptsUs||frame.config.scope!=remote.state.value.scope||frame.config.generation!=decoderConfig?.generation||callbackNanos<renderedNanos){
            // Unknown callback never consumes another issued ticket or ACK.
            active.requestRecovery("StaleControllerCallback");recoverInput();runCatching{remote.requestKeyframe()};return@withLock
        }
        try {
            val ack=remote.rendered(ticket,ptsUs)
            count("rendered")
            nativeTickets.remove(ticket)
            val state=remote.state.value
            if(state.scope!=frame.config.scope||active.ownerId!=decoder?.ownerId||expectedSurfaceEpoch!=synchronized(inputFence){surfaceEpoch}||!foregroundActive||borrowedSurface?.isValid!=true)return@withLock
            val rect=state.clientRect?:return@withLock
            val viewport=RemoteViewport.fit(display.value.viewWidth,display.value.viewHeight,frame.config.visibleWidth,frame.config.visibleHeight)?:return@withLock
            // This adoption follows current-owner OnFrameRendered AND native
            // retained-ticket validation. Queue/release/posting never gets here.
            synchronized(inputFence){
                if(expectedSurfaceEpoch!=surfaceEpoch||!foregroundActive||retired)return@withLock
                mapping.set(RemoteRenderMapping(ownerId,frame,viewport,rect))
                val admission=(active.state.value as? DecoderStatus.Ready)?.admission
                if(admission!=null)renderedMutable.value=RemoteRenderedFrameObservation(ownerId,frame.config.scope,
                    frame.config.generation,frame.ticket,frame.frameId,frame.ptsUs,renderedNanos,callbackNanos,admission,ack)
                val target=remote.target.value
                if(target?.grantActive==true&&inputGrantFence.permits(target.binding)&&target.binding.scope()==frame.config.scope){enabled=true;ghost.activate(target.binding,callbackNanos)}
                if(ack!=null&&target?.grantActive==true&&target.binding==ack.binding){ghost.acknowledge(ack.binding,ack.lastInputSequenceApplied,ack.frameId,ack.ticket,callbackNanos);echoLedger.acknowledge(ack,callbackNanos)}
                echoMutable.value=echoLedger.snapshot(System.nanoTime())
            }
        }catch(_:Exception){active.requestRecovery("NativeTicketRefused");recoverInput();runCatching{remote.requestKeyframe()}}
    }}
    /** MotionEvent is borrowed only during this call; copy bounded actual data.
     * Inverted(2) is unavailable in Android's tool type; Eraser(4) stays distinct. */
    fun touch(event:MotionEvent):Boolean {
        val map=mapping.get()?:return false;val target=remote.target.value?:return false
        if(!enabled||!foregroundActive||!target.grantActive||target.binding.scope()!=map.frame.config.scope)return false
        val fence=synchronized(inputFence){admissionFence}
        val batch=mutableListOf<TouchAdmission>()
        if(event.pointerCount!=1||event.historySize>7)return rejectTouch()
        val tool=event.getToolType(0);if(tool!=MotionEvent.TOOL_TYPE_STYLUS&&tool!=MotionEvent.TOOL_TYPE_ERASER)return rejectTouch()
        val phase=when(event.actionMasked){MotionEvent.ACTION_DOWN->RemotePenPhase.Down;MotionEvent.ACTION_MOVE->RemotePenPhase.Move;MotionEvent.ACTION_UP->RemotePenPhase.Up;MotionEvent.ACTION_HOVER_ENTER,MotionEvent.ACTION_HOVER_MOVE->RemotePenPhase.Hover;MotionEvent.ACTION_HOVER_EXIT,MotionEvent.ACTION_CANCEL->RemotePenPhase.Leave;else->return rejectTouch()}
        synchronized(inputFence){
            if(suppressedGesture){
                if(phase==RemotePenPhase.Up||phase==RemotePenPhase.Leave)suppressedGesture=false
                if(phase!=RemotePenPhase.Down)return false
            }
            if(target.commandBusy){
                if(phase==RemotePenPhase.Down)suppressedGesture=true
                displayMutable.update{it.copy(message="Editor command is finishing. Lift the pen, then draw again.")}
                return false
            }
        }
        val now=System.nanoTime();val uptime=SystemClock.uptimeMillis()
        val flags=(if(event.buttonState and MotionEvent.BUTTON_STYLUS_PRIMARY!=0)1u else 0u) or (if(tool==MotionEvent.TOOL_TYPE_ERASER)4u else 0u)
        val samples=if(phase==RemotePenPhase.Move)event.historySize+1 else 1
        for(index in 0 until samples){val historical=samples>1&&index<event.historySize
            val x=if(historical)event.getHistoricalX(0,index)else event.x;val y=if(historical)event.getHistoricalY(0,index)else event.y
            val point=map.viewport.host(x,y,map.rect)?:return rejectTouch()
            val sampledUptime=if(Build.VERSION.SDK_INT>=34)eventNanos(event,historical,index) else (if(historical)event.getHistoricalEventTime(index)else event.eventTime)*1_000_000L
            // Align to this phone's System.nanoTime without BOOTTIME. Uptime's
            // millisecond upper edge conservatively adds at most 1 ms of age.
            val age=(uptime+1L)*1_000_000L-sampledUptime
            if(age !in 0 until 50_000_000L)return rejectTouch();val sampled=now-age
            val pressure=if(historical)event.getHistoricalPressure(0,index)else event.pressure
            if(!pressure.isFinite()||pressure<0)return rejectTouch()
            val normalized=pressure.coerceAtMost(1f)
            val tilt=if(historical)event.getHistoricalAxisValue(MotionEvent.AXIS_TILT,0,index)else event.getAxisValue(MotionEvent.AXIS_TILT)
            val orientation=if(historical)event.getHistoricalAxisValue(MotionEvent.AXIS_ORIENTATION,0,index)else event.getAxisValue(MotionEvent.AXIS_ORIENTATION)
            if(!tilt.isFinite()||!orientation.isFinite())return rejectTouch()
            val degrees=tilt*180f/Math.PI.toFloat();val tx=(kotlin.math.sin(orientation)*degrees).roundToInt().coerceIn(-90,90);val ty=(-kotlin.math.cos(orientation)*degrees).roundToInt().coerceIn(-90,90)
            val samplePhase=if(historical)RemotePenPhase.Move else phase
            val sample=RemotePenSample(samplePhase,point.first,point.second,(normalized*1024).roundToInt().toUInt(),tx,ty,0u,flags,sampled)
            val contact=samplePhase==RemotePenPhase.Down||samplePhase==RemotePenPhase.Move
            batch+=TouchAdmission(target.binding,sample,GhostPoint(x,y,normalized,sampled,false),contact,fence)
        }
        // Validate the complete borrowed event before admitting any prefix.
        var commandBusy=false
        val admitted=try{synchronized(inputFence){
            if(!enabled||!foregroundActive||fence!=admissionFence||mapping.get()!==map||remote.target.value?.binding!=target.binding)false
            else if(!remote.reservePen(target.binding,batch.size.toUInt())){
                commandBusy=true
                if(phase==RemotePenPhase.Down)suppressedGesture=true
                false
            } else{
                val complete=batch.all{input.trySend(it).isSuccess}
                // Seal before the consumer can admit a partially queued prefix.
                if(!complete){inputGrantFence.invalidate(target.binding);clearInput()}
                complete
            }
        }
        }catch(_:Exception){return rejectTouch()}
        if(commandBusy){
            displayMutable.update{it.copy(message="Editor command is finishing. Lift the pen, then draw again.")}
            return false
        }
        if(!admitted)return rejectTouch()
        return true
    }
    @RequiresApi(34) private fun eventNanos(event:MotionEvent,historical:Boolean,index:Int):Long = if(historical)event.getHistoricalEventTimeNanos(index)else event.eventTimeNanos
    private fun rejectTouch():Boolean{deactivate("owner_pause");return false}
    suspend fun background(){deactivate("background");decoderMutex.withLock{closeDecoder()}}
    override suspend fun retire():Boolean = withContext(NonCancellable){
        if(retired)return@withContext true
        deactivate("owner_pause");input.close();palette.close()
        val done=decoderMutex.withLock{closeDecoder()};if(!done)return@withContext false
        job.cancelAndJoin();borrowedSurface=null;surfaceOwner=null;retired=true;true
    }
}
private fun RemoteVideoScope.decoder()=DecoderScope(connectionEpoch,captureSessionId,sourceGeneration,targetToken,geometryRevision)
private fun RemoteTargetBinding.scope()=RemoteVideoScope(connectionEpoch,captureSessionId,sourceGeneration,targetToken,geometryRevision)
private fun sameConfig(a:RemoteVideoConfig,b:RemoteVideoConfig)=a.scope==b.scope&&a.generation==b.generation&&a.codedWidth==b.codedWidth&&a.codedHeight==b.codedHeight&&a.visibleWidth==b.visibleWidth&&a.visibleHeight==b.visibleHeight&&a.vps.contentEquals(b.vps)&&a.sps.contentEquals(b.sps)&&a.pps.contentEquals(b.pps)
