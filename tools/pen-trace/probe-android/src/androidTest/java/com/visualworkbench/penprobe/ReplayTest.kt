package com.visualworkbench.penprobe

import android.os.SystemClock
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import kotlin.math.abs
import kotlin.math.ceil

/** Physical-device software test. Injected stylus data is never hardware capability evidence. */
@RunWith(AndroidJUnit4::class)
class ReplayTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    @Test fun recorderPreservesHistoryAndAllPointers() {
        val recorder = TraceRecorder()
        recorder.start("synthetic-history", "synthetic", 100, 100)
        val properties = arrayOf(property(3, MotionEvent.TOOL_TYPE_STYLUS), property(7, MotionEvent.TOOL_TYPE_FINGER))
        val coordinates = arrayOf(coords(10f, 20f, 0.3f), coords(30f, 40f, 0.7f))
        val event = MotionEvent.obtain(100, 100, MotionEvent.ACTION_MOVE, 2, properties, coordinates,
            0, MotionEvent.BUTTON_STYLUS_PRIMARY, 1f, 1f, 0, 0, InputDevice.SOURCE_STYLUS, 0x20)
        try {
            coordinates[0].x = 15f
            event.addBatch(107, coordinates, 0)
            recorder.motion(event)
        } finally { event.recycle() }
        val captured = recorder.snapshot().getJSONArray("events").getJSONObject(0)
        assertEquals(4, recorder.count())
        assertEquals(2, captured.getJSONArray("frames").length())
        assertEquals(0x20, captured.getInt("flags") and 0x20)
        assertEquals(10.0, captured.getJSONArray("frames").getJSONObject(0).getJSONArray("pointers")
            .getJSONObject(0).getJSONArray("axes").getDouble(0), 0.0)
        assertEquals(64, captured.getJSONArray("frames").getJSONObject(1).getJSONArray("pointers")
            .getJSONObject(1).getJSONArray("axes").length())
    }

    @Test fun replayAndMeasure() {
        val mode = InstrumentationRegistry.getArguments().getString("traceMode", "synthetic")
        require(mode in setOf("synthetic", "owner"))
        val names = if (mode == "owner") {
            requireNotNull(instrumentation.context.assets.list("")).filter { it.endsWith(".json") }.sorted().also {
                require(it.size in 24..200) { "Owner acceptance requires 24..200 traces; no synthetic substitution" }
            }
        } else listOf("synthetic")
        fun loadTrace(name: String): JSONObject {
            return if (mode == "owner") {
                val bytes = instrumentation.context.assets.open(name).use { input ->
                    val output = java.io.ByteArrayOutputStream()
                    val buffer = ByteArray(8192)
                    var count = input.read(buffer)
                    while (count >= 0) {
                        require(output.size() + count <= 32 * 1024 * 1024) { "Trace exceeds size bound" }
                        output.write(buffer, 0, count)
                        count = input.read(buffer)
                    }
                    output.toByteArray()
                }
                JSONObject(String(bytes, Charsets.UTF_8)).also {
                    require(it.getString("origin") == "owner" && it.getString("stop_reason") == "saved")
                }
            } else syntheticTrace()
        }
        val results = JSONArray()
        ActivityScenario.launch(ProbeActivity::class.java).use { scenario ->
            instrumentation.waitForIdleSync()
            for (name in names) {
                val trace = loadTrace(name)
                var activity: ProbeActivity? = null
                scenario.onActivity { activity = it }
                val probe = requireNotNull(activity)
                val result = runCatching { replay(trace, probe) }
                results.put(result.getOrElse { error -> JSONObject().put("name", trace.optString("name"))
                    .put("passed", false).put("failure", error.message?.take(240) ?: "replay failed") })
                val report = JSONObject().put("mode", mode).put("hardware_capability_evidence", false)
                    .put("results", results).put("complete", results.length() == names.size)
                File(probe.filesDir, "replay-report.json").writeText(report.toString())
                result.getOrThrow()
            }
        }
    }

    @Test fun savedTraceRoundTripsWithoutChangingSamples() {
        ActivityScenario.launch(ProbeActivity::class.java).use { scenario ->
            var activity: ProbeActivity? = null
            scenario.onActivity { activity = it }
            val probe = requireNotNull(activity)
            val source = syntheticTrace()
            val file = probe.writeSnapshot(source)
            try {
                val loaded = JSONObject(file.readText())
                assertEquals(source.getJSONArray("events").toString(), loaded.getJSONArray("events").toString())
                assertEquals("synthetic", loaded.getString("origin"))
            } finally {
                check(file.canonicalFile.parentFile == File(probe.filesDir, "traces").canonicalFile)
                check(file.delete()) { "Task-owned synthetic saved trace cleanup failed" }
            }
        }
    }

    private fun replay(trace: JSONObject, probe: ProbeActivity): JSONObject {
        require(trace.getInt("schema_version") == 1)
        val events = trace.getJSONArray("events")
        require(events.length() in 1..30000)
        // Reject unsupported fields before injecting any part of the trace.
        for (i in 0 until events.length()) {
            val event = events.getJSONObject(i)
            if (event.getString("kind") == "motion") {
                require(event.getInt("action_button") == 0) { "Public MotionEvent construction cannot set actionButton; faithful replay unavailable" }
                require(event.getInt("classification") == 0) { "Classification replay is unavailable on the minimum supported API" }
            } else require(event.getInt("key_code") in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F8) { "Replay permits only Air Action F1-F8 keys" }
        }
        val expected = flatten(events)
        require(expected.isNotEmpty() && expected.size <= 30000)
        val first = events.getJSONObject(0).let { time(it) }
        val last = events.getJSONObject(events.length() - 1).let { time(it, true) }
        require(last >= first && last - first <= 120_000_000_000L) { "Replay limited to 120 seconds per trace" }
        var offset = IntArray(2)
        instrumentation.runOnMainSync {
            require(trace.getJSONObject("viewport").getInt("width") <= probe.surface.width &&
                trace.getJSONObject("viewport").getInt("height") <= probe.surface.height) { "Trace viewport does not fit; replay never rescales data" }
            probe.surface.getLocationOnScreen(offset)
            probe.recorder.start("replay-${trace.getString("name")}".take(64), "replay", probe.surface.width, probe.surface.height)
        }
        val baseMs = SystemClock.uptimeMillis() + 150L
        val shiftNs = baseMs * 1_000_000L - first
        fun mapped(ns: Long) = (ns + shiftNs + 500_000L) / 1_000_000L
        val wallDeviations = mutableListOf<Long>()
        try {
            for (i in 0 until events.length()) {
                val event = events.getJSONObject(i)
                val due = mapped(time(event, true))
                val wait = due - SystemClock.uptimeMillis()
                if (wait > 0) SystemClock.sleep(wait)
                check(probe.focused) { "INCONCLUSIVE: shared-phone focus lost; no other app was stopped" }
                wallDeviations.add(abs(SystemClock.uptimeMillis() * 1_000_000L - (time(event, true) + shiftNs)))
                if (event.getString("kind") == "key") {
                    require(event.getInt("key_code") in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F8) { "Replay permits only Air Action F1-F8 keys" }
                    val key = KeyEvent(mapped(event.getLong("down_time_ns")), due, event.getInt("action"),
                        event.getInt("key_code"), event.getInt("repeat"), event.getInt("meta_state"),
                        0, event.getInt("scan_code"), event.getInt("flags"), event.getInt("source"))
                    check(instrumentation.uiAutomation.injectInputEvent(key, true))
                    continue
                }
                val frames = event.getJSONArray("frames")
                val initial = frames.getJSONObject(0)
                val pointers = initial.getJSONArray("pointers")
                val properties = Array(pointers.length()) { p -> pointers.getJSONObject(p).let { property(it.getInt("id"), it.getInt("tool")) } }
                fun coordinates(frame: JSONObject): Array<MotionEvent.PointerCoords> = Array(properties.size) { p ->
                    val axes = frame.getJSONArray("pointers").getJSONObject(p).getJSONArray("axes")
                    require(axes.length() == 64)
                    MotionEvent.PointerCoords().apply {
                        for (axis in 0..63) {
                            val value = axes.getDouble(axis).toFloat()
                            require(value.isFinite())
                            if (value != 0f) setAxisValue(axis, value)
                        }
                        require(x >= 0 && y >= 0 && x < probe.surface.width && y < probe.surface.height) { "Trace point outside probe canvas" }
                        x += offset[0]; y += offset[1]
                    }
                }
                val input = MotionEvent.obtain(mapped(event.getLong("down_time_ns")), mapped(initial.getLong("time_ns")),
                    event.getInt("action"), properties.size, properties, coordinates(initial), event.getInt("meta_state"),
                    event.getInt("button_state"), event.getDouble("x_precision").toFloat(), event.getDouble("y_precision").toFloat(),
                    0, event.getInt("edge_flags"), event.getInt("source"), event.getInt("flags"))
                try {
                    // Android's public MotionEvent API exposes getActionButton but no setter.
                    // Do not use hidden APIs or silently discard a recorded button transition.
                    require(event.getInt("action_button") == 0) { "Public MotionEvent construction cannot set actionButton; this trace is not replayable faithfully" }
                    for (h in 1 until frames.length()) input.addBatch(mapped(frames.getJSONObject(h).getLong("time_ns")), coordinates(frames.getJSONObject(h)), event.getInt("meta_state"))
                    check(instrumentation.uiAutomation.injectInputEvent(input, true)) { "Input injection rejected" }
                } finally { input.recycle() }
            }
            instrumentation.waitForIdleSync()
        } finally { instrumentation.runOnMainSync { probe.recorder.stop("replay_complete") } }
        val actualTrace = probe.recorder.snapshot()
        val actual = flatten(actualTrace.getJSONArray("events"))
        File(probe.filesDir, "last-replay.json").writeText(actualTrace.toString())
        File(probe.filesDir, "last-replay-source.json").writeText(trace.toString())
        val deviations = mutableListOf<Long>()
        val mismatches = mutableListOf<String>()
        if (expected.size != actual.size) mismatches.add("sample_count: expected ${expected.size}, received ${actual.size}")
        for (i in 0 until minOf(expected.size, actual.size)) {
            val a = actual[i]; val e = expected[i]
            deviations.add(abs(a.getLong("time_ns") - shiftNs - e.getLong("time_ns")))
            for (field in listOf("id", "tool", "action", "button_state", "action_button", "canceled", "source", "meta_state")) {
                if (e.getInt(field) != a.getInt(field)) mismatches.add("sample $i $field")
            }
            for (axis in 0..63) if (abs(e.getJSONArray("axes").getDouble(axis) - a.getJSONArray("axes").getDouble(axis)) > 0.0001) mismatches.add("sample $i axis $axis")
        }
        val expectedKeys = keys(events); val actualKeys = keys(actualTrace.getJSONArray("events"))
        if (expectedKeys.size != actualKeys.size) mismatches.add("key_count")
        for (i in 0 until minOf(expectedKeys.size, actualKeys.size)) {
            deviations.add(abs(actualKeys[i].getLong("time_ns") - shiftNs - expectedKeys[i].getLong("time_ns")))
            for (field in listOf("action", "key_code", "repeat", "meta_state")) if (expectedKeys[i].getInt(field) != actualKeys[i].getInt(field)) mismatches.add("key $i $field")
        }
        val max = deviations.maxOrNull() ?: Long.MAX_VALUE
        val report = JSONObject().put("name", trace.getString("name")).put("expected_samples", expected.size)
            .put("received_samples", actual.size).put("expected_keys", expectedKeys.size).put("received_keys", actualKeys.size)
            .put("timestamp_shift_ns", shiftNs).put("max_timestamp_deviation_ns", max)
            .put("p95_timestamp_deviation_ns", percentile(deviations)).put("max_injection_submission_deviation_ns", wallDeviations.maxOrNull())
            .put("p95_injection_submission_deviation_ns", percentile(wallDeviations)).put("field_mismatches", JSONArray(mismatches.take(100)))
            .put("passed", mismatches.isEmpty() && max <= 1_000_000L)
        File(probe.filesDir, "last-replay-metrics.json").writeText(report.toString())
        assertTrue("Replay mismatch; inspect last-replay-metrics.json", report.getBoolean("passed"))
        return report
    }

    private fun time(event: JSONObject, last: Boolean = false): Long = if (event.getString("kind") == "key") event.getLong("time_ns")
        else event.getJSONArray("frames").let { it.getJSONObject(if (last) it.length() - 1 else 0).getLong("time_ns") }

    private fun keys(events: JSONArray): List<JSONObject> = (0 until events.length()).map { events.getJSONObject(it) }.filter { it.getString("kind") == "key" }

    private fun flatten(events: JSONArray): List<JSONObject> = buildList {
        for (i in 0 until events.length()) {
            val event = events.getJSONObject(i)
            if (event.getString("kind") != "motion") continue
            val frames = event.getJSONArray("frames")
            for (h in 0 until frames.length()) {
                val frame = frames.getJSONObject(h)
                val pointers = frame.getJSONArray("pointers")
                for (p in 0 until pointers.length()) add(JSONObject(pointers.getJSONObject(p).toString())
                    .put("time_ns", frame.getLong("time_ns")).put("action", event.getInt("action"))
                    .put("button_state", event.getInt("button_state")).put("action_button", event.getInt("action_button"))
                    .put("canceled", event.getInt("flags") and 0x20).put("source", event.getInt("source"))
                    .put("meta_state", event.getInt("meta_state")))
            }
        }
    }

    private fun percentile(values: List<Long>): Long = if (values.isEmpty()) 0 else values.sorted()[ceil(values.size * 0.95).toInt() - 1]
    private fun property(id: Int, tool: Int) = MotionEvent.PointerProperties().apply { this.id = id; toolType = tool }
    private fun coords(x: Float, y: Float, pressure: Float) = MotionEvent.PointerCoords().apply {
        this.x = x; this.y = y; this.pressure = pressure; size = 0.1f
        setAxisValue(MotionEvent.AXIS_TILT, 0.4f); setAxisValue(MotionEvent.AXIS_ORIENTATION, 0.7f)
        setAxisValue(MotionEvent.AXIS_DISTANCE, 0.2f)
    }

    private fun syntheticTrace(): JSONObject {
        val recorder = TraceRecorder()
        recorder.start("synthetic-stylus", "synthetic", 220, 220)
        val properties = arrayOf(property(0, MotionEvent.TOOL_TYPE_STYLUS))
        val actions = listOf(MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_EXIT,
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_MOVE, MotionEvent.ACTION_MOVE, MotionEvent.ACTION_UP)
        actions.forEachIndexed { i, action ->
            val time = 1000L + i * 80
            val pointers = arrayOf(coords(40f + i * 15f, 80f + i * 5f, 0.2f + i * 0.1f))
            val event = MotionEvent.obtain(1000, time, action, 1, properties, pointers, 0,
                if (i == 5) MotionEvent.BUTTON_STYLUS_PRIMARY else 0, 1f, 1f, 0, 0, InputDevice.SOURCE_STYLUS, 0)
            try {
                if (i == 4) { pointers[0].x += 2f; event.addBatch(time + 20, pointers, 0) }
                recorder.motion(event)
            } finally { event.recycle() }
        }
        for (code in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F8) {
            val time = 1700L + (code - KeyEvent.KEYCODE_F1) * 80
            recorder.key(KeyEvent(time, time, KeyEvent.ACTION_DOWN, code, 0))
            recorder.key(KeyEvent(time, time + 20, KeyEvent.ACTION_UP, code, 0))
        }
        recorder.stop()
        return recorder.snapshot()
    }
}
