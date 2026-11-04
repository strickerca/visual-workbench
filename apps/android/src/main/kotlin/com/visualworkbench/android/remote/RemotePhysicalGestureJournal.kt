package com.visualworkbench.android.remote

import com.visualworkbench.android.diagnostics.InputDeviceCapability
import com.visualworkbench.shared.RemotePenPhase

/** Observation-only provenance of one borrowed MotionEvent. It never grants,
 * widens or restores input authority, and carries no device name/descriptor.
 *
 * What PhysicalStylus does prove: the event entered this Activity's window
 * dispatch (not a direct View call); its DOWN/UP was HMAC-verified by the
 * system InputDispatcher (API 30+ verifyInputEvent; unavailable on API 29, so
 * never PhysicalStylus there); it came from a non-virtual device id that
 * declares stylus and reports the stylus+touchscreen digitizer source
 * combination; neither the accessibility (0x800) nor generated-gesture (0x8)
 * flag is set; and the owner performed no synthetic dispatch. Journal-level
 * checks (tools/remote-edit/physical_gesture_reports.py) additionally require
 * hardware-shaped axes: varying pressure, declared tilt/orientation/distance
 * present and varying, and batched history.
 *
 * What it does NOT prove: that no privileged injector (shell, UiAutomation,
 * instrumentation) crafted events matching every one of those predicates, or
 * that a human held the pen. Injected events are dispatched and signed by the
 * same InputDispatcher; only their shape, not their signature, differs. */
internal enum class RemoteInputOrigin { PhysicalStylus, Injected, Direct, Unknown }

/** DOWN/UP signature check result. The dispatcher signs only DOWN and UP. */
internal enum class RemoteInputVerification { Verified, Refused, NotSigned, Unavailable, NotDelivered }
internal data class RemoteWindowDelivery(val delivered:Boolean,val verification:RemoteInputVerification)

internal data class RemoteInputProvenance(
    val deviceId:Int,
    /** InputDevice.getSources(); null when the device lookup failed. */
    val deviceSources:Int?,
    /** null when the InputDevice lookup failed; never guessed. */
    val deviceVirtual:Boolean?,
    val declaresStylus:Boolean,
    val external:Boolean?,
    val source:Int,
    val toolType:Int,
    val flags:Int,
    val buttonState:Int,
    val actionButton:Int,
    val classification:Int,
    val historySize:Int,
    val eventTimeNanos:Long,
    val actionMasked:Int,
    /** Witnessed inside the Activity's window dispatch of this same event. */
    val windowDelivered:Boolean,
    val verification:RemoteInputVerification,
)

/** Primitive facts copied from one borrowed MotionEvent and its device. */
internal data class RemoteMotionFacts(
    val deviceId:Int,val deviceSources:Int?,val deviceVirtual:Boolean?,val declaresStylus:Boolean,val external:Boolean?,
    val source:Int,val toolType:Int,val flags:Int,val buttonState:Int,val actionButton:Int,val classification:Int,
    val historySize:Int,val eventTimeNanos:Long,val eventTimeMillis:Long,val actionMasked:Int,
)

/** Hidden MotionEvent.FLAG_IS_ACCESSIBILITY_EVENT: injected by an accessibility service. */
internal const val REMOTE_MOTION_FLAG_ACCESSIBILITY:Int=0x800
/** Hidden MotionEvent.FLAG_IS_GENERATED_GESTURE: synthesized by a gesture generator. */
internal const val REMOTE_MOTION_FLAG_GENERATED_GESTURE:Int=0x8
/** InputDevice.SOURCE_STYLUS (SOURCE_CLASS_POINTER included). */
internal const val REMOTE_SOURCE_STYLUS:Int=0x00004002
/** SOURCE_STYLUS|SOURCE_TOUCHSCREEN: the combination a built-in pen digitizer
 * reports. `input stylus ...` injects SOURCE_STYLUS alone and is refused. */
internal const val REMOTE_SOURCE_PEN_DIGITIZER:Int=0x00005002
internal const val REMOTE_TOOL_TYPE_STYLUS:Int=2
internal const val REMOTE_TOOL_TYPE_ERASER:Int=4
internal const val REMOTE_ACTION_DOWN:Int=0
internal const val REMOTE_ACTION_UP:Int=1

/** Pure classification. PhysicalStylus requires every predicate; anything not
 * proven physical stays Injected, Direct or Unknown and is never promoted. */
internal fun classifyRemoteInput(value:RemoteInputProvenance,syntheticDispatches:Long):RemoteInputOrigin = when {
    // The synthetic producer calls View.dispatchTouchEvent directly.
    !value.windowDelivered->RemoteInputOrigin.Direct
    value.deviceId==-1||value.deviceVirtual==true||
        value.flags and (REMOTE_MOTION_FLAG_ACCESSIBILITY or REMOTE_MOTION_FLAG_GENERATED_GESTURE)!=0||
        value.verification==RemoteInputVerification.Refused->RemoteInputOrigin.Injected
    // A run that also performed direct synthetic dispatch cannot vouch for any event.
    syntheticDispatches!=0L->RemoteInputOrigin.Unknown
    value.deviceVirtual==null||!value.declaresStylus->RemoteInputOrigin.Unknown
    value.toolType!=REMOTE_TOOL_TYPE_STYLUS&&value.toolType!=REMOTE_TOOL_TYPE_ERASER->RemoteInputOrigin.Unknown
    value.deviceSources==null||value.deviceSources and REMOTE_SOURCE_PEN_DIGITIZER!=REMOTE_SOURCE_PEN_DIGITIZER->RemoteInputOrigin.Unknown
    value.source and REMOTE_SOURCE_PEN_DIGITIZER!=REMOTE_SOURCE_PEN_DIGITIZER->RemoteInputOrigin.Unknown
    // Signed actions must carry an actual dispatcher signature (API 30+).
    value.actionMasked==REMOTE_ACTION_DOWN||value.actionMasked==REMOTE_ACTION_UP->
        if(value.verification==RemoteInputVerification.Verified)RemoteInputOrigin.PhysicalStylus else RemoteInputOrigin.Unknown
    else->RemoteInputOrigin.PhysicalStylus
}

internal fun remoteInputProvenance(facts:RemoteMotionFacts,delivery:RemoteWindowDelivery):RemoteInputProvenance =
    RemoteInputProvenance(facts.deviceId,facts.deviceSources,facts.deviceVirtual,facts.declaresStylus,facts.external,facts.source,
        facts.toolType,facts.flags,facts.buttonState,facts.actionButton,facts.classification,facts.historySize,facts.eventTimeNanos,
        facts.actionMasked,delivery.delivered,if(delivery.delivered)delivery.verification else RemoteInputVerification.NotDelivered)

/** Main-thread window-delivery ledger for the integration Activity. A frame is
 * entered only by the Activity's window dispatch; ViewGroup offset copies and
 * synthesized hover enter/exit keep the device id and event time. The owner's
 * direct synthetic View dispatches are counted separately. Not thread-safe:
 * every call happens on the UI thread that dispatches input. */
internal class RemoteWindowDeliveryLedger {
    private class Frame(val depth:Int,val deviceId:Int,val eventTimeMillis:Long,val actionMasked:Int,val verification:RemoteInputVerification)
    private var frame=Frame(0,0,0L,-1,RemoteInputVerification.NotDelivered)
    private val deliveries=java.util.concurrent.atomic.AtomicLong()
    private val synthetic=java.util.concurrent.atomic.AtomicLong()
    val windowDeliveries:Long get()=deliveries.get()
    val syntheticDispatches:Long get()=synthetic.get()
    /** Returns the token to pass to [exit]. */
    fun enter(deviceId:Int,eventTimeMillis:Long,actionMasked:Int,verification:RemoteInputVerification):Any {
        deliveries.incrementAndGet()
        val prior=frame;frame=Frame(prior.depth+1,deviceId,eventTimeMillis,actionMasked,verification);return prior
    }
    fun exit(token:Any){frame=token as Frame}
    fun syntheticDispatch(){synthetic.incrementAndGet()}
    fun delivery(deviceId:Int,eventTimeMillis:Long,actionMasked:Int):RemoteWindowDelivery {
        val current=frame
        if(current.depth<=0||current.deviceId!=deviceId||current.eventTimeMillis!=eventTimeMillis)
            return RemoteWindowDelivery(false,RemoteInputVerification.NotDelivered)
        val signed=actionMasked==REMOTE_ACTION_DOWN||actionMasked==REMOTE_ACTION_UP
        return RemoteWindowDelivery(true,when{
            !signed->RemoteInputVerification.NotSigned
            current.actionMasked==actionMasked->current.verification
            else->RemoteInputVerification.NotSigned
        })
    }
}

internal data class RemotePhysicalJournalEvent(
    val event:Int,val action:Int,val provenance:RemoteInputProvenance,val origin:RemoteInputOrigin,
)
internal data class RemotePhysicalJournalRow(
    val seq:ULong,val event:Int,val phase:RemotePenPhase,val hostX:Int,val hostY:Int,
    val sentPressure:UInt,val rawPressure:Float,val rawTilt:Float?,val rawOrientation:Float?,val rawDistance:Float?,
    val tiltX:Int,val tiltY:Int,val penMask:UInt,val penFlags:UInt,val sampledNanos:Long,val historical:Boolean,
    val origin:RemoteInputOrigin,
)
/** Raw per-sample observation carried with one queued TouchAdmission until the
 * actual native admission supplies its sequence. Holds no input authority. */
internal data class RemotePhysicalSampleTrace(
    val journal:RemotePhysicalGestureJournal,val event:Int,val rawPressure:Float,val rawTilt:Float?,
    val rawOrientation:Float?,val rawDistance:Float?,val historical:Boolean,val origin:RemoteInputOrigin,
)
internal data class RemotePhysicalJournalProgress(
    val accepting:Boolean,val complete:Boolean,val stopReason:String,val admittedStrokes:Int,
    val samples:Int,val events:Int,val pauses:Int,val firstSequence:ULong?,val lastSequence:ULong?,
)
internal data class RemotePhysicalJournalSnapshot(
    val targetStrokes:Int,val hoverOnly:Boolean,val complete:Boolean,val stopReason:String,val admittedStrokes:Int,
    val devices:List<InputDeviceCapability>,val events:List<RemotePhysicalJournalEvent>,
    val samples:List<RemotePhysicalJournalRow>,val pauses:Map<String,Long>,
)
/** Fixed run facts the integration owner adds at serialization. */
internal data class RemotePhysicalJournalHeader(
    val runId:String,val block:String,val deviceModel:String,val sdk:Int,val timestampResolutionNanos:Long,
    val syntheticDispatches:Long,val windowDeliveries:Long,val acknowledgedThrough:ULong,
    val echoObserved:ULong,val echoMissing:ULong,val echoPending:Int,
)

internal val REMOTE_PHYSICAL_BLOCKS:Set<String> = setOf("pressure","hover","barrel","eraser",
    "paint-pressure","paint-hover","paint-barrel","paint-eraser","paint-eraser-tool")
internal fun remotePhysicalHoverOnly(block:String):Boolean = block=="hover"||block=="paint-hover"
internal val REMOTE_OWNER_PAUSE_REASONS:Set<String> = setOf("pointer_batch","tool_type","action","axes","viewport",
    "sample_age","pressure","tilt","admission_exception","admission_refused","input_expired","pen_exception")

/** Bounded, integration-armed journal. Overflow never truncates silently: the
 * journal stops accepting and marks itself incomplete. All state is behind one
 * short local lock that never calls out (no provider, IO, or input fence). */
internal class RemotePhysicalGestureJournal(
    val targetStrokes:Int,
    val hoverOnly:Boolean,
    private val maxEvents:Int=6000,
    private val maxSamples:Int=12000,
    private val maxDevices:Int=16,
    private val maxRanges:Int=64,
) {
    init { require(targetStrokes in 1..64&&maxEvents in 1..65536&&maxSamples in 1..65536&&maxDevices in 1..64&&maxRanges in 1..256) }
    private val lock=Any()
    private val events=ArrayList<RemotePhysicalJournalEvent>()
    private val samples=ArrayList<RemotePhysicalJournalRow>()
    private val devices=LinkedHashMap<Int,InputDeviceCapability>()
    private val pauses=LinkedHashMap<String,Long>()
    private var accepting=true
    private var complete=true
    private var stopReason="recording"
    private var queuedStrokes=0;private var queuedContact=false;private var queuedHover=false
    private var admittedStrokes=0;private var admittedContact=false;private var admittedHover=false

    fun accepting():Boolean = synchronized(lock){accepting}

    /** Lets the caller skip building a census for an already recorded device. */
    fun hasDevice(deviceId:Int):Boolean = synchronized(lock){devices.containsKey(deviceId)}

    fun recordDevice(device:InputDeviceCapability):Unit = synchronized(lock){
        if(!accepting||devices.containsKey(device.deviceId))return
        if(devices.size>=maxDevices){overflow("device_limit");return}
        devices[device.deviceId]=device.copy(ranges=device.ranges.take(maxRanges))
    }

    /** Returns the journal event index and its origin, or null once closed/full. */
    fun recordEvent(provenance:RemoteInputProvenance,action:Int,syntheticDispatches:Long):Pair<Int,RemoteInputOrigin>? = synchronized(lock){
        if(!accepting)return null
        if(events.size>=maxEvents){overflow("event_limit");return null}
        val origin=classifyRemoteInput(provenance,syntheticDispatches)
        val index=events.size
        events+=RemotePhysicalJournalEvent(index,action,provenance,origin)
        index to origin
    }

    /** Called after a complete event batch was queued for native admission. It
     * closes the journal once the owner's requested gesture count was queued. */
    fun queued(phase:RemotePenPhase):Unit = synchronized(lock){
        if(!accepting)return
        when(phase){
            RemotePenPhase.Down->{queuedContact=true;queuedHover=false}
            RemotePenPhase.Move->{}
            RemotePenPhase.Up->if(queuedContact){queuedContact=false;if(!hoverOnly)queuedStrokes++}
            RemotePenPhase.Hover->if(!queuedContact)queuedHover=true
            RemotePenPhase.Leave->{if(hoverOnly&&queuedHover&&!queuedContact)queuedStrokes++;queuedHover=false;queuedContact=false}
        }
        if(queuedStrokes>=targetStrokes){accepting=false;stopReason="target_reached"}
    }

    /** Appends one natively admitted sample. Overflow marks the journal incomplete. */
    fun admitted(row:RemotePhysicalJournalRow):Unit = synchronized(lock){
        if(samples.size>=maxSamples){overflow("sample_limit");return}
        samples+=row
        when(row.phase){
            RemotePenPhase.Down->{admittedContact=true;admittedHover=false}
            RemotePenPhase.Move->{}
            RemotePenPhase.Up->if(admittedContact){admittedContact=false;if(!hoverOnly)admittedStrokes++}
            RemotePenPhase.Hover->if(!admittedContact)admittedHover=true
            RemotePenPhase.Leave->{if(hoverOnly&&admittedHover&&!admittedContact)admittedStrokes++;admittedHover=false;admittedContact=false}
        }
    }

    /** Owner-pause causes from the production rejection paths, counted by fixed label. */
    fun pause(reason:String):Unit = synchronized(lock){
        val label=if(reason in REMOTE_OWNER_PAUSE_REASONS)reason else "unclassified"
        if(pauses.size<32||pauses.containsKey(label))pauses[label]=(pauses[label]?:0L).let{if(it==Long.MAX_VALUE)it else it+1}
    }

    fun stop(reason:String="stopped"):Unit = synchronized(lock){
        if(accepting){accepting=false;stopReason=reason}
    }

    private fun overflow(reason:String){accepting=false;complete=false;stopReason=reason}

    fun progress():RemotePhysicalJournalProgress = synchronized(lock){
        RemotePhysicalJournalProgress(accepting,complete,stopReason,admittedStrokes,samples.size,events.size,
            pauses.values.sum().toInt().coerceAtLeast(0),samples.firstOrNull()?.seq,samples.lastOrNull()?.seq)
    }

    fun snapshot():RemotePhysicalJournalSnapshot = synchronized(lock){
        RemotePhysicalJournalSnapshot(targetStrokes,hoverOnly,complete,stopReason,admittedStrokes,
            devices.values.toList(),events.toList(),samples.toList(),LinkedHashMap(pauses))
    }
}

private fun StringBuilder.jsonString(value:String):StringBuilder {
    append('"')
    for(c in value){
        when{
            c=='"'->append("\\\"")
            c=='\\'->append("\\\\")
            c.code<0x20||c.code==0x7f->append("\\u").append(c.code.toString(16).padStart(4,'0'))
            else->append(c)
        }
    }
    return append('"')
}
private fun StringBuilder.jsonFloat(value:Float?):StringBuilder =
    if(value==null||!value.isFinite())append("null") else append(value.toDouble().toString())
private fun StringBuilder.jsonBool(value:Boolean?):StringBuilder = append(value?.toString()?:"null")

/** Deterministic closed-key JSON. Matches tools/remote-edit/physical_gesture_reports.py. */
internal fun RemotePhysicalJournalSnapshot.toJson(header:RemotePhysicalJournalHeader):String {
    require(header.runId.matches(Regex("[a-f0-9]{32}"))&&header.block in REMOTE_PHYSICAL_BLOCKS)
    require(header.deviceModel.length in 1..64&&header.deviceModel.all{it in 'A'..'Z'||it in 'a'..'z'||it in '0'..'9'||it=='-'||it=='_'})
    val out=StringBuilder(256+samples.size*280+events.size*260)
    out.append("{\"schema\":1,\"kind\":\"remote-physical-gesture-journal\",\"run_id\":").jsonString(header.runId)
    out.append(",\"block\":").jsonString(header.block)
    out.append(",\"target_strokes\":").append(targetStrokes)
    out.append(",\"hover_only\":").append(hoverOnly)
    out.append(",\"device_model\":").jsonString(header.deviceModel)
    out.append(",\"sdk\":").append(header.sdk)
    out.append(",\"clock\":\"android_uptime_ns\",\"timestamp_resolution_ns\":").append(header.timestampResolutionNanos)
    out.append(",\"complete\":").append(complete)
    out.append(",\"stop_reason\":").jsonString(stopReason)
    out.append(",\"synthetic_dispatches\":").append(header.syntheticDispatches)
    out.append(",\"window_deliveries\":").append(header.windowDeliveries)
    out.append(",\"admitted_strokes\":").append(admittedStrokes)
    out.append(",\"acknowledged_through\":").append(header.acknowledgedThrough.toString())
    out.append(",\"echo\":{\"observed\":").append(header.echoObserved.toString())
        .append(",\"missing\":").append(header.echoMissing.toString()).append(",\"pending\":").append(header.echoPending).append('}')
    out.append(",\"input_devices\":[")
    devices.forEachIndexed{i,d->
        if(i>0)out.append(',')
        out.append("{\"device_id\":").append(d.deviceId).append(",\"sources\":").append(d.sources)
            .append(",\"declares_stylus\":").append(d.declaresStylus).append(",\"virtual\":").append(d.virtual)
            .append(",\"external\":").append(d.external).append(",\"ranges\":[")
        d.ranges.forEachIndexed{j,r->
            if(j>0)out.append(',')
            out.append("{\"axis\":").append(r.axis).append(",\"source\":").append(r.source)
                .append(",\"min\":").jsonFloat(r.min).append(",\"max\":").jsonFloat(r.max)
                .append(",\"flat\":").jsonFloat(r.flat).append(",\"fuzz\":").jsonFloat(r.fuzz)
                .append(",\"resolution\":").jsonFloat(r.resolution).append('}')
        }
        out.append("]}")
    }
    out.append("],\"events\":[")
    events.forEachIndexed{i,e->
        if(i>0)out.append(',')
        val p=e.provenance
        out.append("{\"event\":").append(e.event).append(",\"action\":").append(e.action)
            .append(",\"device_id\":").append(p.deviceId).append(",\"device_virtual\":").jsonBool(p.deviceVirtual)
            .append(",\"declares_stylus\":").append(p.declaresStylus).append(",\"external\":").jsonBool(p.external)
            .append(",\"device_sources\":").append(p.deviceSources?.toString()?:"null")
            .append(",\"source\":").append(p.source).append(",\"tool_type\":").append(p.toolType)
            .append(",\"flags\":").append(p.flags).append(",\"button_state\":").append(p.buttonState)
            .append(",\"action_button\":").append(p.actionButton).append(",\"classification\":").append(p.classification)
            .append(",\"history_size\":").append(p.historySize).append(",\"event_time_ns\":").append(p.eventTimeNanos)
            .append(",\"window_delivered\":").append(p.windowDelivered).append(",\"verification\":").jsonString(p.verification.name)
            .append(",\"origin\":").jsonString(e.origin.name).append('}')
    }
    out.append("],\"samples\":[")
    samples.forEachIndexed{i,s->
        if(i>0)out.append(',')
        out.append("{\"seq\":").append(s.seq.toString()).append(",\"event\":").append(s.event)
            .append(",\"phase\":").jsonString(s.phase.name).append(",\"host_x\":").append(s.hostX).append(",\"host_y\":").append(s.hostY)
            .append(",\"sent_pressure\":").append(s.sentPressure.toString()).append(",\"raw_pressure\":").jsonFloat(s.rawPressure)
            .append(",\"raw_tilt\":").jsonFloat(s.rawTilt).append(",\"raw_orientation\":").jsonFloat(s.rawOrientation)
            .append(",\"raw_distance\":").jsonFloat(s.rawDistance).append(",\"tilt_x\":").append(s.tiltX).append(",\"tilt_y\":").append(s.tiltY)
            .append(",\"pen_mask\":").append(s.penMask.toString()).append(",\"pen_flags\":").append(s.penFlags.toString())
            .append(",\"sampled_ns\":").append(s.sampledNanos).append(",\"historical\":").append(s.historical)
            .append(",\"origin\":").jsonString(s.origin.name).append('}')
    }
    out.append("],\"owner_pauses\":[")
    pauses.entries.forEachIndexed{i,(reason,count)->
        if(i>0)out.append(',')
        out.append("{\"reason\":").jsonString(reason).append(",\"count\":").append(count).append('}')
    }
    out.append("]}")
    return out.toString()
}
