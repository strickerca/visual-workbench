package com.visualworkbench.android.airactions

import android.view.MotionEvent

/** Read only the fields needed for contact/barrel observation. The original
 * MotionEvent is neither copied into remote input nor changed/recycled. */
internal fun airRemoteMotionFrame(event: MotionEvent): AirRemoteMotionFrame {
    require(event.pointerCount in 0..16)
    val phase = when (event.actionMasked) {
        MotionEvent.ACTION_DOWN -> AirRemoteMotionPhase.Down
        MotionEvent.ACTION_POINTER_DOWN -> AirRemoteMotionPhase.PointerDown
        MotionEvent.ACTION_MOVE -> AirRemoteMotionPhase.Move
        MotionEvent.ACTION_UP -> AirRemoteMotionPhase.Up
        MotionEvent.ACTION_POINTER_UP -> AirRemoteMotionPhase.PointerUp
        MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE -> AirRemoteMotionPhase.Hover
        MotionEvent.ACTION_HOVER_EXIT -> AirRemoteMotionPhase.Leave
        MotionEvent.ACTION_CANCEL -> AirRemoteMotionPhase.Cancel
        else -> AirRemoteMotionPhase.Other
    }
    val pointer = if (event.pointerCount == 0) -1 else event.getPointerId(event.actionIndex)
    val pointers = (0 until event.pointerCount).map { index ->
        val tool = event.getToolType(index)
        AirRemoteMotionPointer(event.getPointerId(index), tool == MotionEvent.TOOL_TYPE_STYLUS || tool == MotionEvent.TOOL_TYPE_ERASER)
    }
    return AirRemoteMotionFrame(phase, pointer, pointers,
        event.buttonState and MotionEvent.BUTTON_STYLUS_PRIMARY != 0, event.eventTime,
        event.flags and 0x20 != 0) // FLAG_CANCELED, including canceled POINTER_UP.
}
