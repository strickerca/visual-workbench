package com.visualworkbench.android.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.visualworkbench.android.diagnostics.DiagnosticSurface
import com.visualworkbench.android.diagnostics.DiagnosticsController
import com.visualworkbench.android.diagnostics.PenPathMetrics
import com.visualworkbench.android.diagnostics.PenTiming
import com.visualworkbench.android.editor.PenPathToggle
import com.visualworkbench.android.editor.PenPathToggles
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import kotlinx.coroutines.delay

@Composable
internal fun DiagnosticsScreen(controller: DiagnosticsController, penPath: PenPathToggles = PenPathToggles(), onBack: () -> Unit) {
    var surface by remember { mutableStateOf<DiagnosticSurface?>(null) }
    var showTimings by remember { mutableStateOf(false) }
    var name by remember { mutableStateOf("line-slow-01") }
    val reportExport = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/json")) {
        controller.completeReportExport(it)
    }
    Column(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row {
            TextButton(onClick = { controller.background(); onBack() }, modifier = Modifier.heightIn(min = 48.dp)) { Text("← Projects") }
            Text("Pen diagnostics", style = MaterialTheme.typography.h6, modifier = Modifier.padding(12.dp))
        }
        Text("Raw input, pressure, axes, hover, buttons and cancellation. Generic stylus profile; no Samsung module required.", style = MaterialTheme.typography.body2)
        OutlinedTextField(name, { value -> name = value.lowercase().filter { it in 'a'..'z' || it in '0'..'9' || it == '-' }.take(64) },
            label = { Text("Trace name") }, singleLine = true, enabled = !controller.recording && !controller.busy, modifier = Modifier.fillMaxWidth())
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button(onClick = { surface?.let { it.clear(); controller.start(name, it.width, it.height) } },
                enabled = !controller.recording && !controller.busy && name.matches(Regex("[a-z0-9][a-z0-9-]{0,63}")), modifier = Modifier.heightIn(min = 48.dp)) { Text("Record") }
            OutlinedButton(onClick = { controller.save() }, enabled = !controller.busy && (controller.recording || controller.samples > 0), modifier = Modifier.heightIn(min = 48.dp)) { Text("Save trace") }
            Text("${controller.samples} samples", modifier = Modifier.padding(top = 14.dp))
        }
        Text(controller.message, style = MaterialTheme.typography.body2)
        Text(controller.lastPressure, style = MaterialTheme.typography.caption)
        OutlinedButton(onClick = controller::prepareReport, enabled = !controller.reportBusy,
            modifier = Modifier.heightIn(min = 48.dp)) { Text("Preview diagnostic report") }
        controller.reportMessage?.let { Text(it, style = MaterialTheme.typography.body2) }
        OutlinedButton(onClick = { showTimings = true }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Pen path timings") }
        AndroidView(factory = { context -> DiagnosticSurface(context, controller).also { surface = it } }, modifier = Modifier.fillMaxWidth().weight(1f))
        Text("Draw callbacks exclude compositor presentation. PERF-001 and S Pen acceptance remain separate physical tests.", style = MaterialTheme.typography.caption)
    }
    if (showTimings) AlertDialog(onDismissRequest = { showTimings = false }, title = { Text("Pen path timings") },
        text = { Column(Modifier.heightIn(max = 480.dp).verticalScroll(rememberScrollState())) { PenPathTimings(controller.pen, penPath) } },
        confirmButton = { TextButton(onClick = { showTimings = false }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Close") } })
    controller.reportPreview?.let { preview ->
        AlertDialog(onDismissRequest = controller::dismissReportPreview,
            title = { Text("Diagnostic report preview") },
            text = {
                Column(Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Only counters, timings, fixed event codes and the app's own fixed-wording reasons for recent failures are included. Raw traces, images, names and identifiers stay out.")
                    Text("The text below is exactly what will be saved.")
                    Text(preview.jsonText, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.caption)
                    controller.reportMessage?.let { Text(it) }
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    if (controller.beginReportExport()) {
                        try { reportExport.launch("visual-workbench-diagnostics.json") }
                        catch (_: Exception) { controller.reportPickerFailed() }
                    }
                }, enabled = !controller.reportBusy, modifier = Modifier.heightIn(min = 48.dp)) { Text("Save report…") }
            },
            dismissButton = {
                TextButton(onClick = controller::dismissReportPreview, enabled = !controller.reportBusy,
                    modifier = Modifier.heightIn(min = 48.dp)) { Text("Close") }
            })
    }
}

/** Middle (p50) and slow-end (p95) timings over the most recent pen samples and
 * strokes, refreshed once a second while shown. Durations and counts only.
 * The 25 ms target (PERF-001) is for pen contact to ink on the screen; these
 * app-side timings end when the frame is handed to the display. */
@Composable
internal fun PenPathTimings(metrics: PenPathMetrics, penPath: PenPathToggles) {
    val switchesOn = PenPathToggle.entries.filter { penPath[it] }.map { it.key }
    var tick by remember { mutableStateOf(0) }
    LaunchedEffect(metrics) { while (true) { delay(1_000); tick++ } }
    val clipboard = LocalClipboardManager.current
    val rows = remember(tick, metrics) { PenTiming.entries.map { it to metrics.summary(it) }.filter { it.second.count > 0 } }
    Text("Pen path timings", style = MaterialTheme.typography.h6)
    if (rows.isEmpty()) Text("No timings yet. Draw a few strokes on the canvas or in the remote view, then come back.", style = MaterialTheme.typography.body2)
    rows.forEach { (timing, s) ->
        Text("${timing.label}: typical ${PenPathMetrics.ms(s.p50Micros)} ms · slow end ${PenPathMetrics.ms(s.p95Micros)} ms · worst ${PenPathMetrics.ms(s.maxMicros)} ms (${s.count} of ${s.total})",
            style = MaterialTheme.typography.body2)
    }
    val downs = remember(tick, metrics) { metrics.penDowns() }
    if (downs.accepted != 0L || downs.refusedTotal != 0L) Text(PenPathMetrics.penDownsLine(downs), style = MaterialTheme.typography.body2)
    Text(if (switchesOn.isEmpty()) "Developer switches on: none" else "Developer switches on: ${switchesOn.joinToString(", ")}", style = MaterialTheme.typography.caption)
    Text("Typical is the middle value (p50); slow end is the value 95 in 100 stay under (p95). They end when the frame is handed to the display, not at the glass.", style = MaterialTheme.typography.caption)
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        OutlinedButton(onClick = { metrics.clear(); tick++ }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Clear timings") }
        OutlinedButton(onClick = { clipboard.setText(AnnotatedString(metrics.report(switchesOn))) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Copy timings") }
    }
}
