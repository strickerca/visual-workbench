package com.visualworkbench.strokespike

import android.os.SystemClock
import org.json.JSONArray
import org.json.JSONObject
import java.util.ArrayDeque

/** Bounded callback observations. They are never presented as photon latency. */
internal class StrokeMetrics {
    private val records = ArrayDeque<JSONObject>()
    private var dropped = 0
    @Synchronized fun record(engine: String, phase: String, inputMs: Long, observedNs: Long = SystemClock.uptimeMillis() * 1_000_000L) {
        if (records.size == 4096) { records.removeFirst(); dropped++ }
        records.add(JSONObject().put("engine", engine).put("phase", phase)
            .put("delay_ns", (observedNs - inputMs * 1_000_000L).coerceAtLeast(0L)))
    }
    @Synchronized fun report(): JSONObject = JSONObject().put("schema", 1)
        .put("measurement", "input timestamp to callback; excludes display presentation")
        .put("clock_resolution_ns", 1_000_000L)
        .put("hardware_pen_acceptance", false).put("dropped_records", dropped)
        .put("records", JSONArray(records.toList()))
    @Synchronized fun observed(engine: String, phase: String): Boolean = records.any {
        it.getString("engine") == engine && it.getString("phase") == phase
    }
}
