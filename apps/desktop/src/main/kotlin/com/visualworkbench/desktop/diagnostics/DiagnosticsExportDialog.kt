package com.visualworkbench.desktop.diagnostics

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.DiagnosticEventCode
import com.visualworkbench.shared.PreparedDiagnosticsReport
import kotlinx.coroutines.*
import java.awt.EventQueue
import java.awt.FileDialog
import java.awt.Frame
import java.nio.ByteBuffer
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.util.UUID
import java.util.concurrent.atomic.AtomicInteger
import kotlin.math.roundToLong

private const val MAX_METRIC = 9_007_199_254_740_991L
private const val EXPORT_NANOS = 5_000_000_000L

internal fun measuredDiagnosticValue(value: Long?): Long? = value?.takeIf { it in 0L..MAX_METRIC }

/** This is measured echo RTT, never a pen-to-display latency estimate. */
internal fun measuredEchoRoundTripMillis(samples: UInt, value: Double?): Long? {
    if (samples == 0u || value == null || !value.isFinite() || value < 0.0 || value > MAX_METRIC.toDouble()) return null
    return measuredDiagnosticValue(value.roundToLong())
}

/** Cancellation has one clear boundary: once commit wins, wait for its actual result. */
internal class DiagnosticPublicationGate {
    private val state = AtomicInteger(0)
    fun cancel(): Boolean = state.compareAndSet(0, 1)
    fun isCancelled(): Boolean = state.get() == 1
    fun beginCommit(): Boolean = state.compareAndSet(0, 2)
    fun finishCommit() { check(state.compareAndSet(2, 3)) }
}

internal enum class DiagnosticWriteResult { Saved, Cancelled, Failed }

/** One bounded payload and one CREATE_NEW sibling. Pending ownership is retained
 * if native IO cannot retire. Hard-link publication preserves existing files and
 * never exposes partial bytes at the user-selected name. Unsupported volumes fail.
 */
internal class DiagnosticPublication(
    report: PreparedDiagnosticsReport,
    private val destination: Path,
    val gate: DiagnosticPublicationGate = DiagnosticPublicationGate(),
    private val nowNanos: () -> Long = System::nanoTime,
) {
    private val bytes = report.jsonText.toByteArray(Charsets.UTF_8).also { check(it.size in 1..65_536) }
    private var temporary: DiagnosticOwnedFile? = null
    val cleanupPending: Boolean get() = temporary != null
    var published: Boolean = false
        private set

    fun write(): DiagnosticWriteResult {
        check(!published && temporary == null)
        val started = nowNanos()
        fun admitted() {
            if (gate.isCancelled()) throw PublicationCancelled()
            check(nowNanos() - started in 0L..EXPORT_NANOS)
        }
        val result = try {
            admitted()
            check(destination.isAbsolute && destination == destination.normalize())
            check(destination.fileName.toString().endsWith(".json", ignoreCase = true))
            val parent = checkNotNull(destination.parent)
            check(Files.isDirectory(parent, NOFOLLOW_LINKS) && parent.toRealPath() == parent)
            check(!Files.exists(destination, NOFOLLOW_LINKS))
            admitted()
            val path = parent.resolve(".vw-diagnostics-${UUID.randomUUID()}.tmp")
            val owned = createDiagnosticOwnedFile(path)
            temporary = owned // Adopt the actual channel/path before any later read can fail.
            owned.captureIdentity()
            val buffer = ByteBuffer.wrap(bytes)
            var attempts = 0
            while (buffer.hasRemaining()) {
                admitted()
                check(attempts++ < bytes.size)
                val end = minOf(buffer.position() + 4_096, bytes.size)
                buffer.limit(end)
                check(owned.write(buffer) > 0)
                buffer.limit(bytes.size)
            }
            admitted()
            owned.force()
            owned.finishWriting()
            admitted()
            if (!gate.beginCommit()) throw PublicationCancelled()
            try {
                owned.publish(bytes.size.toLong()) {
                    Files.createLink(destination, path)
                    published = true
                }
            } finally { gate.finishCommit() }
            DiagnosticWriteResult.Saved
        } catch (_: PublicationCancelled) { DiagnosticWriteResult.Cancelled }
        catch (_: Exception) { DiagnosticWriteResult.Failed }
        finally { cleanup() }
        return result
    }

    /** Only this retained file identity may be deleted. A refused cleanup stays held. */
    fun cleanup(): Boolean {
        val owned = temporary ?: return true
        return try {
            if (!owned.cleanup()) return false
            temporary = null
            true
        } catch (_: Exception) { false }
    }

    private class PublicationCancelled : Exception()
}

internal enum class DiagnosticsExportPhase { Preview, Choosing, Writing, CleanupPending }

/** UI-dispatcher owner. The chooser, focus lease, job and owned sibling remain
 * here until actual retirement; a wait timeout never disposes their ownership.
 */
internal class DiagnosticsExportOwner(
    private val scope: CoroutineScope,
    private val recordEvent: (DiagnosticEventCode) -> Unit,
) {
    private class Picker(val owner: Frame, private val focusLease: AutoCloseable) {
        private var chooser: FileDialog? = null
        var cancelled = false
        var handlerFinished = false
        var retired = false
            private set
        fun create(): FileDialog {
            check(!retired && chooser == null)
            return FileDialog(owner, "Save diagnostic report", FileDialog.SAVE).also { chooser = it }
        }
        fun close() {
            if (retired) return
            chooser?.dispose() // Failed disposal keeps the chooser and focus lease for retry.
            chooser = null
            focusLease.close()
            retired = true
        }
    }

    var report by mutableStateOf<PreparedDiagnosticsReport?>(null)
        private set
    var phase by mutableStateOf(DiagnosticsExportPhase.Preview)
        private set
    var notice by mutableStateOf<String?>(null)
        private set
    private var picker: Picker? = null
    private var producer: Job? = null
    private var publication: DiagnosticPublication? = null
    private var closeRequested = false
    val held: Boolean get() = report != null
    val previewVisible: Boolean get() = phase != DiagnosticsExportPhase.Choosing && picker == null && held
    val busy: Boolean get() = phase != DiagnosticsExportPhase.Preview

    fun prepare(value: PreparedDiagnosticsReport) {
        check(EventQueue.isDispatchThread() && !held && picker == null && producer == null && publication == null)
        report = value
        closeRequested = false
        phase = DiagnosticsExportPhase.Preview
        notice = null
    }

    fun choose(owner: Frame, directory: Path, retainFocus: () -> AutoCloseable, current: () -> Boolean) {
        check(EventQueue.isDispatchThread())
        val frozen = report ?: return
        settle()
        if (busy || producer != null || publication != null || picker != null || closeRequested || !currentChoice(current)) return
        val request = try { Picker(owner, retainFocus()) }
        catch (_: Exception) { notice = "The save picker could not open."; record(DiagnosticEventCode.ExportFailed); return }
        picker = request
        phase = DiagnosticsExportPhase.Choosing // Removes the Compose preview before the next AWT tick.
        notice = null
        fun complete(chosen: Path?, failed: Boolean) {
            request.handlerFinished = true
            try { request.close() } catch (_: Exception) { /* Keep the actual pending picker. */ }
            if (!request.retired) {
                phase = DiagnosticsExportPhase.CleanupPending
                notice = "Save picker cleanup is pending. Retry Close before leaving."
                record(DiagnosticEventCode.ExportFailed)
                return
            }
            if (picker === request) picker = null
            if (chosen != null && !request.cancelled && !closeRequested && held && report === frozen && currentChoice(current)) {
                startWrite(frozen, chosen)
            } else {
                phase = DiagnosticsExportPhase.Preview
                notice = if (failed) "The report was not saved. Choose a new .json file in a local folder." else "Save cancelled. No report file was written."
                record(if (failed) DiagnosticEventCode.ExportFailed else DiagnosticEventCode.ExportCancelled)
                settle()
            }
        }
        try {
            EventQueue.invokeLater {
                var chosen: Path? = null
                var failed = false
                try {
                    if (!request.cancelled && !closeRequested && held && report === frozen && picker === request &&
                        !request.retired && owner.isDisplayable && currentChoice(current)) {
                        val chooser = request.create()
                        chooser.directory = directory.toString()
                        chooser.file = "visual-workbench-diagnostics.json"
                        chooser.isVisible = true
                        if (!request.cancelled && !request.retired && !closeRequested && owner.isDisplayable && currentChoice(current)) {
                            chooser.file?.let { name ->
                                check(name.endsWith(".json", ignoreCase = true))
                                chosen = Path.of(chooser.directory, name).toAbsolutePath().normalize()
                            }
                        }
                    }
                } catch (_: Exception) { failed = true }
                finally { complete(chosen, failed) }
            }
        } catch (_: Exception) { complete(null, true) }
    }

    private fun startWrite(frozen: PreparedDiagnosticsReport, destination: Path) {
        val work = DiagnosticPublication(frozen, destination)
        publication = work
        phase = DiagnosticsExportPhase.Writing
        tracked {
            val result = withContext(Dispatchers.IO) { work.write() }
            // Adopt the actual result before a log callback can fail.
            phase = if (work.cleanupPending) DiagnosticsExportPhase.CleanupPending else DiagnosticsExportPhase.Preview
            notice = when {
                work.cleanupPending && work.published -> "The complete report was saved. Temporary-file cleanup is pending."
                work.cleanupPending -> "No report was saved. Temporary-file cleanup is pending."
                result == DiagnosticWriteResult.Saved -> "The exact preview was saved."
                result == DiagnosticWriteResult.Cancelled -> "Save cancelled. No report file was written."
                else -> "The report was not saved. Choose a new .json file in a local folder."
            }
            record(when (result) {
                DiagnosticWriteResult.Saved -> DiagnosticEventCode.ExportSucceeded
                DiagnosticWriteResult.Cancelled -> DiagnosticEventCode.ExportCancelled
                DiagnosticWriteResult.Failed -> DiagnosticEventCode.ExportFailed
            })
        }
    }

    private fun tracked(body: suspend () -> Unit) {
        check(producer == null)
        // Register before start. NonCancellable protects only settlement of this
        // retained child; it creates no independent scope or detached producer.
        val job = scope.launch(start = CoroutineStart.LAZY) { withContext(NonCancellable) { body() } }
        producer = job
        job.invokeOnCompletion { EventQueue.invokeLater { settle() } }
        job.start()
    }

    fun cancelExport() {
        check(EventQueue.isDispatchThread())
        publication?.gate?.cancel()
        notice = "Waiting for the export's actual result…"
    }

    /** A quick cancellation request; no pending job, picker or file is forgotten. */
    fun requestClose(): Boolean {
        check(EventQueue.isDispatchThread())
        closeRequested = true
        publication?.gate?.cancel()
        picker?.let { request ->
            request.cancelled = true
            try { request.close() } catch (_: Exception) { /* Retain until actual disposal. */ }
        }
        settle()
        if (producer == null && publication?.cleanupPending == true) {
            val work = checkNotNull(publication)
            tracked { withContext(Dispatchers.IO) { work.cleanup() } }
        }
        settle()
        return !held
    }

    suspend fun closeWithin(timeoutMillis: Long = 5_000): Boolean {
        require(timeoutMillis in 1..5_000)
        if (requestClose()) return true
        val retired = withTimeoutOrNull(timeoutMillis) {
            while (held) {
                producer?.join()
                settle()
                if (held) delay(25)
            }
            true
        } ?: false
        if (!retired) notice = "Export shutdown is pending. The app still holds its owners. Retry Quit after it settles."
        return retired
    }

    private fun settle() {
        check(EventQueue.isDispatchThread())
        producer?.takeIf { it.isCompleted }?.let { producer = null }
        picker?.takeIf { it.handlerFinished && it.retired }?.let { picker = null }
        if (producer == null && publication?.cleanupPending == false) publication = null
        if (closeRequested && picker == null && producer == null && publication == null) {
            report = null
            phase = DiagnosticsExportPhase.Preview
        }
    }

    private fun record(code: DiagnosticEventCode) {
        try { recordEvent(code) } catch (_: Exception) { /* Logging cannot discard actual export ownership. */ }
    }

    private fun currentChoice(current: () -> Boolean): Boolean = try { current() } catch (_: Exception) { false }
}

@Composable
internal fun DiagnosticsExportDialog(owner: DiagnosticsExportOwner, onSave: () -> Unit, onClose: () -> Unit) {
    if (!owner.previewVisible) return
    val frozen = owner.report ?: return
    AlertDialog(onDismissRequest = { if (!owner.busy) onClose() }, title = { Text("Diagnostic report") }, text = {
        Column(Modifier.widthIn(max = 620.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Review the exact report before saving. It contains timing, memory and app events. Missing measurements are null.")
            Text("Echo RTT measures the connection round trip; it is not drawing latency.", style = MaterialTheme.typography.caption)
            SelectionContainer {
                Text(frozen.jsonText, Modifier.fillMaxWidth().heightIn(max = 300.dp).verticalScroll(rememberScrollState()),
                    fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.caption)
            }
            Text("Choose a new .json filename in a local folder. Existing files are preserved.", style = MaterialTheme.typography.caption)
            owner.notice?.let { Text(it, style = MaterialTheme.typography.caption) }
            if (owner.phase == DiagnosticsExportPhase.Writing) LinearProgressIndicator(Modifier.fillMaxWidth())
        }
    }, confirmButton = { Button(onClick = onSave, enabled = !owner.busy) { Text("Save report…") } }, dismissButton = {
        Row {
            if (owner.phase == DiagnosticsExportPhase.Writing) TextButton(onClick = owner::cancelExport) { Text("Cancel export") }
            TextButton(onClick = onClose, enabled = owner.phase != DiagnosticsExportPhase.Writing) {
                Text(if (owner.phase == DiagnosticsExportPhase.CleanupPending) "Retry Close" else "Close")
            }
        }
    })
}
