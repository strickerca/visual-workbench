package com.visualworkbench.android.diagnostics

import com.visualworkbench.shared.DiagnosticEventCode
import com.visualworkbench.shared.PreparedDiagnosticsReport
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import java.io.OutputStream

/** Takes ownership of the newly opened stream and writes only the frozen preview. */
internal fun writePreparedDiagnosticsReport(report: PreparedDiagnosticsReport, open: () -> OutputStream) {
    open().use { output ->
        output.write(report.jsonText.toByteArray(Charsets.UTF_8))
        output.flush()
    }
}

/** A structured child retains the stream until IO and owner-side settlement finish. */
internal fun CoroutineScope.launchPreparedReportWriter(
    report: PreparedDiagnosticsReport,
    open: () -> OutputStream,
    settle: (DiagnosticEventCode) -> Unit,
): Job = launch(start = CoroutineStart.UNDISPATCHED) {
    // Protect both result adoption and owner-side settlement. Protecting IO alone
    // would let prompt cancellation discard its successful result on return.
    // This remains inside the retained launch child; a bounded wait never detaches it.
    withContext(NonCancellable) {
        var outcome = DiagnosticEventCode.ExportFailed
        try {
            outcome = withContext(Dispatchers.IO) {
                try {
                    writePreparedDiagnosticsReport(report, open)
                    DiagnosticEventCode.ExportSucceeded
                } catch (_: Exception) { DiagnosticEventCode.ExportFailed }
            }
        } finally { settle(outcome) }
    }
}

/** A wait deadline does not cancel or transfer ownership of a pending writer. */
internal suspend fun awaitPreparedReportWriter(writer: Job, waitMillis: Long = 5_000L): Boolean {
    require(waitMillis in 0L..5_000L)
    if (writer.isCompleted) return true
    return withTimeoutOrNull(waitMillis) { writer.join(); true } == true || writer.isCompleted
}
