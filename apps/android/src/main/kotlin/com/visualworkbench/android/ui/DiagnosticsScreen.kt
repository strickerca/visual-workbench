package com.visualworkbench.android.ui

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.visualworkbench.android.diagnostics.DiagnosticSurface
import com.visualworkbench.android.diagnostics.DiagnosticsController

@Composable
internal fun DiagnosticsScreen(controller: DiagnosticsController, onBack: () -> Unit) {
    var surface by remember { mutableStateOf<DiagnosticSurface?>(null) }
    var name by remember { mutableStateOf("line-slow-01") }
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
        AndroidView(factory = { context -> DiagnosticSurface(context, controller).also { surface = it } }, modifier = Modifier.fillMaxWidth().weight(1f))
        Text("Draw callbacks exclude compositor presentation. PERF-001 and S Pen acceptance remain separate physical tests.", style = MaterialTheme.typography.caption)
    }
}
