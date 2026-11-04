package com.visualworkbench.android.remote

import android.os.Build
import android.view.InputDevice
import android.view.MotionEvent
import androidx.annotation.RequiresApi

/** Supplied only by an explicit integration owner. A witness observes delivery;
 * it never admits, grants, or replays input. */
internal interface RemoteInputDeliveryWitness {
    /** True only while the Activity window is dispatching this same event. */
    fun windowDelivered(event:MotionEvent):Boolean
    /** Count of direct synthetic View dispatches performed in this owner. */
    val syntheticDispatches:Long
    /** Count of events observed entering the Activity window dispatch. */
    val windowDeliveries:Long
}

/** Copies bounded provenance from the borrowed event. No name, descriptor,
 * vendor/product ID or other device identity is read. */
internal fun captureRemoteInputProvenance(event:MotionEvent,windowDelivered:Boolean):RemoteInputProvenance {
    val device:InputDevice?=event.device
    val time=if(Build.VERSION.SDK_INT>=34)provenanceEventNanos(event) else event.eventTime*1_000_000L
    return RemoteInputProvenance(event.deviceId,device?.isVirtual,device?.supportsSource(InputDevice.SOURCE_STYLUS)==true,
        device?.isExternal,event.source,event.getToolType(0),event.flags,event.buttonState,event.actionButton,
        event.classification,event.historySize,time,windowDelivered)
}
@RequiresApi(34) private fun provenanceEventNanos(event:MotionEvent):Long = event.eventTimeNanos
