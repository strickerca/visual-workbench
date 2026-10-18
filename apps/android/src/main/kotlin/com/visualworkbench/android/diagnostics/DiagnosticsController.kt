package com.visualworkbench.android.diagnostics

import android.content.Context
import android.net.Uri
import android.os.SystemClock
import android.view.MotionEvent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.visualworkbench.shared.DiagnosticEventCode
import com.visualworkbench.shared.DiagnosticMetrics
import com.visualworkbench.shared.DiagnosticPlatform
import com.visualworkbench.shared.PreparedDiagnosticsReport
import com.visualworkbench.shared.PrivateDiagnosticLog
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException
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
    private val resolver = context.applicationContext.contentResolver
    private val sessionStarted = SystemClock.elapsedRealtime()
    private val diagnosticLog = PrivateDiagnosticLog().also { it.append(0L, DiagnosticEventCode.AppStarted) }
    var reportPreview by mutableStateOf<PreparedDiagnosticsReport?>(null)
        private set
    var reportBusy by mutableStateOf(false)
        private set
    var reportMessage by mutableStateOf<String?>(null)
        private set
    var reportShutdownPending by mutableStateOf(false)
        private set
    private var pendingReport: PreparedDiagnosticsReport? = null
    private var reportSaveJob: Job? = null
    private var reportClosing = false

    private fun appendDiagnostic(code: DiagnosticEventCode) {
        diagnosticLog.append(SystemClock.elapsedRealtime() - sessionStarted, code)
    }

    fun start(name: String, width: Int, height: Int, synthetic: Boolean = false) {
        if (busy || recording) return
        if (width <= 0 || height <= 0) {
            message = "Wait for the diagnostic canvas to appear."
            appendDiagnostic(DiagnosticEventCode.TraceRejected)
            return
        }
        recorder.start(name, if (synthetic) "synthetic" else "owner", width, height)
        recording = true; samples = 0; message = "Recording locally. Save before switching apps."
        appendDiagnostic(DiagnosticEventCode.TraceStarted)
    }
    fun event(event: MotionEvent) {
        try { recorder.motion(event) }
        catch (_: org.json.JSONException) {
            recorder.stop("invalid_axis_value"); recording = false
            message = "Input contained an invalid axis value. The incomplete trace is retained."
            appendDiagnostic(DiagnosticEventCode.TraceRejected)
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
            appendDiagnostic(if (result.isSuccess) DiagnosticEventCode.TraceSaved else DiagnosticEventCode.TraceRejected)
        }
    }
    fun prepareReport() {
        if (reportClosing || reportBusy) return
        val runtime = Runtime.getRuntime()
        val heapBytes = (runtime.totalMemory() - runtime.freeMemory()).takeIf { it >= 0L }
        appendDiagnostic(DiagnosticEventCode.ReportPrepared)
        reportPreview = diagnosticLog.prepare(DiagnosticPlatform.Android,
            DiagnosticMetrics(heapUsedBytes = heapBytes, traceSampleCount = recorder.count()))
        reportMessage = null
    }

    fun dismissReportPreview() {
        if (!reportBusy) reportPreview = null
    }

    /** Retain the exact preview before opening an external document picker. */
    fun beginReportExport(): Boolean {
        if (reportClosing || reportBusy) return false
        val report = reportPreview ?: return false
        pendingReport = report
        reportBusy = true
        reportMessage = "Choose where to save this report."
        return true
    }

    fun reportPickerFailed() {
        if (pendingReport == null) return
        pendingReport = null
        reportBusy = false
        appendDiagnostic(DiagnosticEventCode.ExportFailed)
        reportMessage = "The document picker could not open. Your preview is still available."
    }

    fun completeReportExport(uri: Uri?) {
        val report = pendingReport ?: return
        pendingReport = null
        if (reportClosing) { reportBusy = false; return }
        if (uri == null) {
            reportBusy = false
            appendDiagnostic(DiagnosticEventCode.ExportCancelled)
            reportMessage = "Report export cancelled. Nothing was written."
            return
        }
        if (uri.scheme != "content") {
            reportBusy = false
            appendDiagnostic(DiagnosticEventCode.ExportFailed)
            reportMessage = "The selected destination could not be used. Your preview is still available."
            return
        }
        reportMessage = "Saving the previewed report…"
        reportSaveJob = scope.launch(start = CoroutineStart.UNDISPATCHED) {
            var outcome = DiagnosticEventCode.ExportFailed
            try {
                outcome = withContext(Dispatchers.IO) {
                    try {
                        writePreparedDiagnosticsReport(report) {
                            resolver.openOutputStream(uri, "wt") ?: throw IOException("Report destination unavailable")
                        }
                        DiagnosticEventCode.ExportSucceeded
                    } catch (_: Exception) { DiagnosticEventCode.ExportFailed }
                }
            } finally {
                reportShutdownPending = false
                reportBusy = false
                appendDiagnostic(outcome)
                reportMessage = if (outcome == DiagnosticEventCode.ExportSucceeded) "Report saved."
                    else "Report export failed. Your preview is still available; the selected file may be incomplete."
            }
        }
    }

    fun background() {
        if (recorder.recording) {
            appendDiagnostic(DiagnosticEventCode.TraceInterrupted)
            save("interrupted_background")
        }
    }

    suspend fun awaitSave() {
        reportClosing = true
        if (pendingReport != null) {
            pendingReport = null
            reportBusy = false
            appendDiagnostic(DiagnosticEventCode.ExportCancelled)
            reportMessage = "Report export cancelled. Nothing was written."
        }
        saveJob?.join()
        val writer = reportSaveJob
        if (writer != null) {
            val settled = awaitPreparedReportWriter(writer)
            if (!settled && !writer.isCompleted) {
                reportShutdownPending = true
                reportMessage = "The selected destination is still processing. Its write and cleanup remain pending."
                // Keep this Job, its preview and busy state. Parent cancellation requests
                // cancellation; the owned IO child must still return and close its stream.
            }
        }
    }
}
