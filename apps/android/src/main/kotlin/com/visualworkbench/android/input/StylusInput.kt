package com.visualworkbench.android.input

import android.view.InputDevice
import android.view.MotionEvent

/** No device names, serials, descriptors, vendor SDK or device-specific calibration. */
internal enum class PointerTool { Pen, Eraser, Finger, Mouse, Palm, Unknown }

internal data class StylusCapabilities(
    val pressure: Boolean,
    val tilt: Boolean,
    val orientation: Boolean,
    val hover: Boolean,
    val eraser: Boolean,
    val buttons: Boolean,
    val samsungActions: Boolean = false,
)

internal data class PenSample(
    val pointerId: Int,
    val tool: PointerTool,
    val x: Double,
    val y: Double,
    val timeMs: Long,
    val rawPressure: Float,
    val pressure: Float,
    val tilt: Float?,
    val orientation: Float?,
    val buttons: Int,
)

internal data class PointerFrame(val timeMs: Long, val pointers: List<PenSample>)
internal data class PointerPacket(
    val action: Int,
    val actionPointer: Int,
    val canceled: Boolean,
    val downTimeMs: Long,
    val frames: List<PointerFrame>,
    val capabilities: StylusCapabilities,
)

/** Optional hardware integration cannot change the generic MotionEvent mapping. */
internal interface SamsungActions {
    val available: Boolean
    fun onKey(keyCode: Int, down: Boolean): Boolean
}

internal object NoSamsungActions : SamsungActions {
    override val available = false
    override fun onKey(keyCode: Int, down: Boolean) = false
}

internal class StylusInput(private val samsung: SamsungActions = NoSamsungActions) {
    fun snapshot(event: MotionEvent): PointerPacket {
        require(event.pointerCount in 1..16 && event.historySize <= 511) { "Input batch exceeded its limit" }
        val device = event.device
        fun range(axis: Int): InputDevice.MotionRange? =
            device?.getMotionRange(axis, event.source) ?: device?.getMotionRange(axis)
        val pressureRange = range(MotionEvent.AXIS_PRESSURE)
        val tiltRange = range(MotionEvent.AXIS_TILT)
        val orientationRange = range(MotionEvent.AXIS_ORIENTATION)
        val tools = (0 until event.pointerCount).map { tool(event.getToolType(it)) }
        val hasPen = tools.any { it == PointerTool.Pen || it == PointerTool.Eraser }
        fun observed(axis: Int): Boolean = (0 until event.pointerCount).any { pointer ->
            event.getAxisValue(axis, pointer) != 0f || (0 until event.historySize).any { history ->
                event.getHistoricalAxisValue(axis, pointer, history) != 0f
            }
        }
        val hasTilt = hasPen && (tiltRange != null || observed(MotionEvent.AXIS_TILT))
        val hasOrientation = hasPen && (orientationRange != null || observed(MotionEvent.AXIS_ORIENTATION))
        val hovering = event.actionMasked in setOf(MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_EXIT)
        val capabilities = StylusCapabilities(
            pressure = pressureRange != null,
            tilt = hasTilt,
            orientation = hasOrientation,
            hover = hasPen && (hovering || range(MotionEvent.AXIS_DISTANCE) != null),
            eraser = PointerTool.Eraser in tools,
            buttons = hasPen && (event.buttonState != 0 || device?.supportsSource(InputDevice.SOURCE_STYLUS) == true),
            samsungActions = samsung.available,
        )
        val frames = (0..event.historySize).map { history ->
            val historical = history < event.historySize
            val time = if (historical) event.getHistoricalEventTime(history) else event.eventTime
            require(time >= 0) { "Invalid input timestamp" }
            val pointers = (0 until event.pointerCount).map { index ->
                fun axis(value: Int): Float = if (historical) event.getHistoricalAxisValue(value, index, history)
                    else event.getAxisValue(value, index)
                val raw = axis(MotionEvent.AXIS_PRESSURE)
                val x = axis(MotionEvent.AXIS_X)
                val y = axis(MotionEvent.AXIS_Y)
                require(raw.isFinite() && x.isFinite() && y.isFinite()) { "Invalid input sample" }
                val normalized = if (pressureRange != null && pressureRange.min.isFinite() &&
                    pressureRange.max.isFinite() && pressureRange.max > pressureRange.min) {
                    ((raw - pressureRange.min) / (pressureRange.max - pressureRange.min)).coerceIn(0f, 1f)
                } else raw.coerceIn(0f, 1f)
                val tilt = if (hasTilt) axis(MotionEvent.AXIS_TILT).also { require(it.isFinite()) } else null
                val orientation = if (hasOrientation) axis(MotionEvent.AXIS_ORIENTATION).also { require(it.isFinite()) } else null
                PenSample(event.getPointerId(index), tools[index], x.toDouble(), y.toDouble(), time,
                    raw, normalized, tilt, orientation, event.buttonState)
            }
            PointerFrame(time, pointers)
        }
        require(frames.zipWithNext().all { (a, b) -> a.timeMs <= b.timeMs }) { "Out-of-order input batch" }
        return PointerPacket(event.actionMasked, event.getPointerId(event.actionIndex),
            event.actionMasked == MotionEvent.ACTION_CANCEL || event.flags and 0x20 != 0,
            event.downTime, frames, capabilities)
    }

    private fun tool(value: Int): PointerTool = when (value) {
        MotionEvent.TOOL_TYPE_STYLUS -> PointerTool.Pen
        MotionEvent.TOOL_TYPE_ERASER -> PointerTool.Eraser
        MotionEvent.TOOL_TYPE_FINGER -> PointerTool.Finger
        MotionEvent.TOOL_TYPE_MOUSE -> PointerTool.Mouse
        5 -> PointerTool.Palm // TOOL_TYPE_PALM is reported by newer platform input stacks.
        else -> PointerTool.Unknown
    }
}
