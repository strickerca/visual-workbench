package com.visualworkbench.android.diagnostics

import android.os.Build
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import org.json.JSONArray
import org.json.JSONObject

/** Immutable JSON event snapshots: never keep a pooled MotionEvent beyond dispatch. */
class TraceRecorder {
    private val events = mutableListOf<JSONObject>()
    private val devices = linkedMapOf<Int, JSONObject>()
    private val timing = mutableListOf<JSONObject>()
    private var sampleCount = 0
    private var name = "unstarted"
    private var origin = "owner"
    private var viewport = JSONObject()
    private var stopReason = "not_started"
    @Volatile var recording = false
        private set
    @Volatile var generation = 0L
        private set

    @Synchronized fun start(traceName: String, traceOrigin: String, width: Int, height: Int) {
        require(traceName.matches(Regex("[a-z0-9][a-z0-9-]{0,63}")))
        require(traceOrigin in setOf("owner", "synthetic", "replay"))
        require(width > 0 && height > 0)
        generation++
        events.clear(); devices.clear(); timing.clear(); sampleCount = 0
        name = traceName; origin = traceOrigin
        viewport = JSONObject().put("width", width).put("height", height)
        stopReason = "recording"; recording = true
    }

    @Synchronized fun stop(reason: String = "saved") { recording = false; stopReason = reason }

    @Synchronized fun motion(event: MotionEvent): Boolean {
        if (!recording) return false
        val count = (event.historySize + 1) * event.pointerCount
        // A stopped/overflowed trace is explicitly incomplete, never silently truncated.
        if (sampleCount + count > 30000 || events.size >= 30000) {
            stop("sample_limit"); return false
        }
        event.device?.let { device ->
            if (!devices.containsKey(device.id)) devices[device.id] = capability(device)
        }
        val frames = JSONArray()
        for (history in 0..event.historySize) {
            val historical = history < event.historySize
            val time = if (Build.VERSION.SDK_INT >= 34) {
                if (historical) event.getHistoricalEventTimeNanos(history) else event.eventTimeNanos
            } else {
                (if (historical) event.getHistoricalEventTime(history) else event.eventTime) * 1_000_000L
            }
            val pointers = JSONArray()
            for (pointer in 0 until event.pointerCount) {
                val axes = JSONArray()
                for (axis in 0..63) {
                    axes.put((if (historical) event.getHistoricalAxisValue(axis, pointer, history)
                        else event.getAxisValue(axis, pointer)).toDouble())
                }
                pointers.put(JSONObject().put("id", event.getPointerId(pointer))
                    .put("tool", event.getToolType(pointer)).put("axes", axes))
            }
            frames.put(JSONObject().put("time_ns", time).put("historical", historical).put("pointers", pointers))
        }
        events.add(JSONObject().put("kind", "motion").put("action", event.action)
            .put("button_state", event.buttonState).put("action_button", event.actionButton)
            .put("flags", event.flags).put("meta_state", event.metaState)
            .put("source", event.source).put("device_id", event.deviceId)
            .put("down_time_ns", event.downTime * 1_000_000L).put("edge_flags", event.edgeFlags)
            .put("x_precision", event.xPrecision.toDouble()).put("y_precision", event.yPrecision.toDouble())
            .put("classification", event.classification).put("received_ns", System.nanoTime())
            .put("frames", frames))
        sampleCount += count
        return true
    }

    @Synchronized fun key(event: KeyEvent) {
        if (!recording) return
        if (events.size >= 30000) { stop("event_limit"); return }
        // Key characters and input-device names/descriptors are deliberately never collected.
        events.add(JSONObject().put("kind", "key").put("action", event.action)
            .put("key_code", event.keyCode).put("scan_code", event.scanCode)
            .put("repeat", event.repeatCount).put("flags", event.flags).put("meta_state", event.metaState)
            .put("device_id", event.deviceId).put("source", event.source)
            .put("time_ns", event.eventTime * 1_000_000L).put("down_time_ns", event.downTime * 1_000_000L))
    }

    @Synchronized fun timing(session: Long, kind: String, inputTime: Long, callbackTime: Long, frameTime: Long? = null) {
        if (recording && session == generation && timing.size < 30000) timing.add(JSONObject().put("kind", kind)
            .put("input_ns", inputTime).put("callback_ns", callbackTime)
            .put("frame_ns", frameTime ?: JSONObject.NULL))
    }

    @Synchronized fun snapshot(): JSONObject = JSONObject()
        .put("schema_version", 1).put("name", name).put("origin", origin)
        .put("device", JSONObject().put("manufacturer", Build.MANUFACTURER)
            .put("model", Build.MODEL).put("sdk", Build.VERSION.SDK_INT))
        .put("clock", "android_uptime_ns").put("timestamp_resolution_ns", if (Build.VERSION.SDK_INT >= 34) 1 else 1_000_000)
        .put("viewport", JSONObject(viewport.toString())).put("stop_reason", stopReason)
        .put("input_devices", JSONArray(devices.values.toList()))
        .put("events", JSONArray(events.toList())).put("timing", JSONArray(timing.toList()))

    @Synchronized fun count(): Int = sampleCount
    @Synchronized fun atLimit(): Boolean = stopReason == "sample_limit" || stopReason == "event_limit"

    private fun capability(device: InputDevice): JSONObject {
        val ranges = JSONArray()
        device.motionRanges.forEach { range -> ranges.put(JSONObject().put("axis", range.axis)
            .put("source", range.source).put("min", range.min.toDouble()).put("max", range.max.toDouble())
            .put("flat", range.flat.toDouble()).put("fuzz", range.fuzz.toDouble()).put("resolution", range.resolution.toDouble())) }
        return JSONObject().put("device_id", device.id).put("sources", device.sources)
            .put("declares_stylus", device.supportsSource(InputDevice.SOURCE_STYLUS))
            .put("virtual", device.isVirtual).put("ranges", ranges)
    }
}
