package com.visualworkbench.android.diagnostics

import android.content.Context
import android.os.SystemClock
import android.view.MotionEvent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.util.UUID

internal class DiagnosticsController(context: Context, private val scope: CoroutineScope) {
    private val directory = File(context.applicationContext.filesDir, "traces")
    val recorder = TraceRecorder()
    var recording by mutableStateOf(false)
        private set
    var samples by mutableStateOf(0)
        private set
    var busy by mutableStateOf(false)
        private set
    var message by mutableStateOf("Record only test marks. Traces stay private on this phone.")
        private set
    var lastPressure by mutableStateOf("No input yet")
        private set
    private var lastUpdate = 0L
    private var saveJob: Job? = null

    fun start(name: String, width: Int, height: Int, synthetic: Boolean = false) {
        if (busy || recording) return
        if (width <= 0 || height <= 0) { message = "Wait for the diagnostic canvas to appear."; return }
        recorder.start(name, if (synthetic) "synthetic" else "owner", width, height)
        recording = true; samples = 0; message = "Recording locally. Save before switching apps."
    }
    fun event(event: MotionEvent) {
        try { recorder.motion(event) }
        catch (_: org.json.JSONException) {
            recorder.stop("invalid_axis_value"); recording = false
            message = "Input contained an invalid axis value. The incomplete trace is retained."
            return
        }
        val now = SystemClock.uptimeMillis()
        if (now - lastUpdate >= 100 || event.actionMasked == MotionEvent.ACTION_UP) {
            lastUpdate = now; samples = recorder.count()
            recording = recorder.recording
            val raw = event.pressure
            val range = event.device?.getMotionRange(MotionEvent.AXIS_PRESSURE, event.source)
            val normalized = if (range != null && range.max > range.min) ((raw - range.min) / (range.max - range.min)).coerceIn(0f, 1f) else raw.coerceIn(0f, 1f)
            lastPressure = "Raw pressure ${"%.3f".format(raw)} · normalized ${"%.3f".format(normalized)}"
            if (recorder.atLimit()) message = "Trace limit reached. Save this explicitly incomplete trace."
        }
    }
    fun save(reason: String = "saved") {
        if (busy) return
        if (recorder.count() == 0) {
            recorder.stop(reason); recording = false
            message = "Recording stopped. There were no samples to save."
            return
        }
        recorder.stop(if (recorder.atLimit()) "sample_limit" else reason)
        recording = false; busy = true; message = "Saving locally…"
        saveJob = scope.launch {
            val result = runCatching { withContext(Dispatchers.IO) {
                check(directory.isDirectory || directory.mkdirs()) { "Trace folder unavailable" }
                val snapshot = recorder.snapshot()
                val file = File(directory, "${snapshot.getString("name")}-${UUID.randomUUID()}.json")
                check(file.createNewFile()) { "Trace name collision" }
                try { file.outputStream().buffered().use { it.write(snapshot.toString().toByteArray(Charsets.UTF_8)) } }
                catch (error: Exception) { file.delete(); throw error }
                file.name
            } }
            busy = false
            message = result.fold({ "Saved $it in this app's private trace folder." }, { "Save failed. Your trace remains in memory; try again." })
        }
    }
    fun background() { if (recorder.recording) save("interrupted_background") }
    suspend fun awaitSave() { saveJob?.join() }
}
