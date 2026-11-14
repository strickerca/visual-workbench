package com.visualworkbench.android.remote

import android.hardware.input.InputManager
import android.os.Build
import android.view.InputDevice
import android.view.MotionEvent
import androidx.annotation.RequiresApi

/** Supplied only by an explicit integration owner. A witness observes delivery;
 * it never admits, grants, or replays input. */
internal interface RemoteInputDeliveryWitness {
    /** Whether the Activity window is dispatching this same event, and the
     * dispatcher-signature result recorded for its DOWN/UP at window entry. */
    fun delivery(event:MotionEvent):RemoteWindowDelivery
    /** Count of direct synthetic View dispatches performed in this owner. */
    val syntheticDispatches:Long
    /** Count of events observed entering the Activity window dispatch. */
    val windowDeliveries:Long
}

/** Copies bounded facts from the borrowed event. No name, descriptor,
 * vendor/product ID or other device identity is read. */
internal fun remoteMotionFacts(event:MotionEvent):RemoteMotionFacts {
    val device:InputDevice?=event.device
    val nanos=if(Build.VERSION.SDK_INT>=34)provenanceEventNanos(event) else event.eventTime*1_000_000L
    return RemoteMotionFacts(event.deviceId,device?.sources,device?.isVirtual,device?.supportsSource(InputDevice.SOURCE_STYLUS)==true,
        device?.isExternal,event.source,event.getToolType(0),event.flags,event.buttonState,event.actionButton,event.classification,
        event.historySize,nanos,event.eventTime,event.actionMasked)
}

internal fun captureRemoteInputProvenance(event:MotionEvent,witness:RemoteInputDeliveryWitness):RemoteInputProvenance =
    remoteInputProvenance(remoteMotionFacts(event),witness.delivery(event))

/** InputDispatcher signs only DOWN/UP. API 29 has no verifier, so a signed
 * action there stays Unavailable and is never classified PhysicalStylus. */
internal fun verifyRemoteInputEvent(manager:InputManager?,event:MotionEvent):RemoteInputVerification {
    val action=event.actionMasked
    if(action!=MotionEvent.ACTION_DOWN&&action!=MotionEvent.ACTION_UP)return RemoteInputVerification.NotSigned
    if(Build.VERSION.SDK_INT<30||manager==null)return RemoteInputVerification.Unavailable
    return if(verifiedBySystem(manager,event))RemoteInputVerification.Verified else RemoteInputVerification.Refused
}
@RequiresApi(30) private fun verifiedBySystem(manager:InputManager,event:MotionEvent):Boolean {
    val verified=manager.verifyInputEvent(event)?:return false
    return verified.deviceId==event.deviceId&&verified.source==event.source&&verified.eventTimeNanos/1_000_000L==event.eventTime
}
@RequiresApi(34) private fun provenanceEventNanos(event:MotionEvent):Long = event.eventTimeNanos
