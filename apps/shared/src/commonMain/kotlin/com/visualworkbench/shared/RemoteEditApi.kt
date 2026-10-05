package com.visualworkbench.shared

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow

public data class RemotePhysicalRect(public val x:Int,public val y:Int,public val width:UInt,public val height:UInt)
public data class RemoteVideoScope(public val connectionEpoch:ULong,public val captureSessionId:String,public val sourceGeneration:ULong,public val targetToken:String,public val geometryRevision:UInt)
public data class RemoteTargetBinding(public val connectionEpoch:ULong,public val captureSessionId:String,public val sourceGeneration:ULong,public val targetToken:String,public val geometryRevision:UInt,public val inputSessionId:String)
public data class RemoteCanvasFocusProof(public val provider:String,public val runtimeIdHash:String,public val processId:UInt,public val targetToken:String,public val geometryRevision:UInt,public val sampledQpc100ns:ULong,public val focusedClassName:String,public val focusedControlType:Int,public val canvasRectHost:RemotePhysicalRect,public val shortcutProfileDigest:String)
public data class RemoteTargetState(public val binding:RemoteTargetBinding,public val identity:RemoteEditorIdentity,public val verifiedActions:Set<RemoteEditorAction>,public val focusProof:RemoteCanvasFocusProof?,public val grantActive:Boolean,public val pauseReason:String?,public val commandBusy:Boolean=false)
public enum class RemoteCommandStatus { Injected,Refused,SealedPartial }
public data class RemoteCommandReceipt(public val binding:RemoteTargetBinding,public val action:RemoteEditorAction,public val inputSequence:ULong,public val acceptedQpc100ns:ULong,public val status:RemoteCommandStatus,public val reason:String?)
public interface RemoteCommandDispatcher { public val target:StateFlow<RemoteTargetState?>;public suspend fun dispatch(binding:RemoteTargetBinding,action:RemoteEditorAction):RemoteCommandReceipt }
public data class RemoteWindowCandidate(public val window:ULong,public val processId:UInt,public val processCreated:ULong,public val label:String,public val executableName:String)
public enum class RemoteEditStatus { Unavailable,Disconnected,Selecting,Viewing,PendingFocus,PendingGrant,Controlling,Paused,Sealed,Closing,Closed }
public data class RemoteEditState(public val revision:ULong,public val status:RemoteEditStatus,public val scope:RemoteVideoScope?,public val clientRect:RemotePhysicalRect?,public val target:RemoteTargetState?,public val reason:String?,public val encoderDescription:String?,public val destinationLabel:String?=null)
public data class RemoteVideoConfig(public val scope:RemoteVideoScope,public val generation:ULong,public val codedWidth:Int,public val codedHeight:Int,public val visibleWidth:Int,public val visibleHeight:Int,public val vps:ByteArray,public val sps:ByteArray,public val pps:ByteArray)
/** Native admission retains ticket metadata. Decoder callbacks cannot supply ACKs. */
public data class RemoteVideoFrame(public val ticket:ULong,public val config:RemoteVideoConfig,public val frameId:ULong,public val ptsUs:Long,public val keyframe:Boolean,public val bytes:ByteArray)
public enum class RemotePenPhase { Hover,Down,Move,Up,Leave }
public data class RemotePenSample(public val phase:RemotePenPhase,public val hostX:Int,public val hostY:Int,public val pressure:UInt,public val tiltX:Int,public val tiltY:Int,public val rotation:UInt,public val penFlags:UInt,public val sampledLocalNanos:Long)
public data class RemoteInputAdmission(public val binding:RemoteTargetBinding,public val sequence:ULong,public val sampledLocalNanos:Long)
public data class RemoteRenderedAcknowledgment(public val binding:RemoteTargetBinding,public val frameId:ULong,public val ticket:ULong,public val lastInputSequenceApplied:ULong)
/** Borrowed link capability. A failed close retains its native owner and blocks
 * link destruction; retry close until actual helper/decoder retirement completes. */
/** Consumer invalidation is immediate. retire reports success only after its
 * actual decoder/surface/palette workers have joined; false retains the owner. */
public interface RemoteConsumerOwner { public fun invalidate();public suspend fun retire():Boolean }
public interface WorkbenchRemoteEdit:RemoteCommandDispatcher {
    /** Binding replacement/revocation invalidates every registered consumer
     * synchronously before the replacement target/state becomes visible.
     * Consumers must not repeat cancellation from an asynchronous state watcher. */
    public fun registerConsumer(owner:RemoteConsumerOwner)
    public fun invalidateConsumers()
    /** Immediate bounded native revocation, without awaiting OS/codec joins.
     * Actual close remains pending until every retained owner has retired. */
    public fun seal(reason:String)
    public val state:StateFlow<RemoteEditState>
    public val frames:Flow<RemoteVideoFrame>
    public suspend fun windows():List<RemoteWindowCandidate>
    public suspend fun select(candidate:RemoteWindowCandidate)
    /** Owner's PC action: attempt exact-target focus, revalidate and create fresh
     * native grant. Pending focus never advertises an active grant. */
    public suspend fun grant()
    public suspend fun requestControl()
    public suspend fun pause(reason:String)
    public suspend fun requestKeyframe()
    /** Short native state-only reservation, called before the bounded phone queue.
     * False refuses new samples while a finite command refreshes authority. */
    public fun reservePen(binding:RemoteTargetBinding,samples:UInt):Boolean
    public suspend fun pen(binding:RemoteTargetBinding,sample:RemotePenSample):RemoteInputAdmission
    /** Drop a retained native ticket without acknowledging its frame or input. */
    public suspend fun discardFrame(ticket:ULong)
    /** Only a current decoder-owner render callback may request validation.
     * Unknown ticket/PTS never consumes another retained frame. The native
     * owner validates current scope, grant and its retained frame metadata. */
    public suspend fun rendered(ticket:ULong,ptsUs:Long):RemoteRenderedAcknowledgment?
    public suspend fun close()
}
