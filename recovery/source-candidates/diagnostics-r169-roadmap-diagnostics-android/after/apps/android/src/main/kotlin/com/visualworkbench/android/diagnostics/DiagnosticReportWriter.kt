package com.visualworkbench.android.diagnostics

import com.visualworkbench.shared.PreparedDiagnosticsReport
import kotlinx.coroutines.Job
import kotlinx.coroutines.withTimeoutOrNull
import java.io.OutputStream

/** Takes ownership of the newly opened stream and writes only the frozen preview. */
internal fun writePreparedDiagnosticsReport(report: PreparedDiagnosticsReport, open: () -> OutputStream) {
    open().use { output ->
        output.write(report.jsonText.toByteArray(Charsets.UTF_8))
        output.flush()
    }
}

/** A wait deadline does not cancel or transfer ownership of a pending writer. */
internal suspend fun awaitPreparedReportWriter(writer: Job, waitMillis: Long = 5_000L): Boolean {
    require(waitMillis in 0L..5_000L)
    if (writer.isCompleted) return true
    return withTimeoutOrNull(waitMillis) { writer.join(); true } == true || writer.isCompleted
}
