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

@Composable
internal fun DiagnosticsScreen(controller: DiagnosticsController, onBack: () -> Unit) {
    var surface by remember { mutableStateOf<DiagnosticSurface?>(null) }
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
        AndroidView(factory = { context -> DiagnosticSurface(context, controller).also { surface = it } }, modifier = Modifier.fillMaxWidth().weight(1f))
        Text("Draw callbacks exclude compositor presentation. PERF-001 and S Pen acceptance remain separate physical tests.", style = MaterialTheme.typography.caption)
    }
    controller.reportPreview?.let { preview ->
        AlertDialog(onDismissRequest = controller::dismissReportPreview,
            title = { Text("Diagnostic report preview") },
            text = {
                Column(Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Only counters and fixed event codes are included. Raw traces, images, names and identifiers stay out.")
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
