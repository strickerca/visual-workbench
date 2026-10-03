package com.visualworkbench.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.*
import androidx.compose.runtime.Composable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import com.visualworkbench.android.editor.CameraCapture
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

internal data class ProjectCard(val id: String, val title: String, val detail: String, val modified: String)

@Composable
internal fun ProjectsScreen(projects: List<ProjectCard>, busy: Boolean, onOpenImage: () -> Unit, onNewCanvas: () -> Unit,
                            onOpen: (String) -> Unit, onSettings: () -> Unit, onDiagnostics: () -> Unit, onPairing: () -> Unit,
                            onFiles: () -> Unit, onCamera: () -> Unit, cameraCaptures: List<CameraCapture>,
                            onRetryCamera: (String) -> Unit, onDiscardCamera: (String) -> Unit, onSaveCamera: (String) -> Unit) {
    Column(Modifier.fillMaxSize().padding(horizontal = 20.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Row(Modifier.fillMaxWidth().padding(top = 20.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("VISUAL WORKBENCH", style = MaterialTheme.typography.overline, color = MaterialTheme.colors.primary)
                Text("Your projects", style = MaterialTheme.typography.h4)
            }
            TextButton(onClick = onSettings, modifier = Modifier.heightIn(min = 48.dp)) { Text("Settings") }
        }
        Text("Images, ideas, and the marks that connect them. Everything here stays available offline.", style = MaterialTheme.typography.body1)
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button(onClick = onOpenImage, enabled = !busy, modifier = Modifier.weight(1f).heightIn(min = 52.dp), shape = RoundedCornerShape(12.dp)) { Text("Photos") }
            OutlinedButton(onClick = onNewCanvas, enabled = !busy, modifier = Modifier.weight(1f).heightIn(min = 52.dp), shape = RoundedCornerShape(12.dp)) { Text("New canvas") }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedButton(onClick = onFiles, enabled = !busy, modifier = Modifier.weight(1f).heightIn(min = 48.dp)) { Text("Browse files") }
            OutlinedButton(onClick = onCamera, enabled = !busy, modifier = Modifier.weight(1f).heightIn(min = 48.dp)) { Text("Take photo") }
        }
        if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        if (cameraCaptures.isNotEmpty()) Column(Modifier.fillMaxWidth().heightIn(max = 200.dp).verticalScroll(rememberScrollState())) {
            Text("Retained camera originals", style = MaterialTheme.typography.subtitle1)
            Text("An import did not finish. These full-resolution originals stay on this phone until imported or discarded.", style = MaterialTheme.typography.caption)
            cameraCaptures.forEachIndexed { index, capture ->
                Text("Camera original ${index + 1} · ${capture.bytes / (1024 * 1024)} MiB", style = MaterialTheme.typography.caption)
                Row {
                    TextButton(onClick = { onRetryCamera(capture.token) }, enabled = !busy, modifier = Modifier.heightIn(min = 48.dp)) { Text("Retry") }
                    TextButton(onClick = { onSaveCamera(capture.token) }, enabled = !busy, modifier = Modifier.heightIn(min = 48.dp)) { Text("Save original") }
                    TextButton(onClick = { onDiscardCamera(capture.token) }, enabled = !busy, modifier = Modifier.heightIn(min = 48.dp)) { Text("Discard") }
                }
            }
        }
        if (projects.isEmpty()) {
            Surface(Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp)) {
                Column(Modifier.padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text("A clear place to begin", style = MaterialTheme.typography.h6)
                    Text("Open a photo or start with a blank canvas. Use your pen to draw; use your fingers to move around.")
                    Text("Originals stay intact. Every mark can be edited or undone.", style = MaterialTheme.typography.body2, color = MaterialTheme.colors.secondary)
                }
            }
            Spacer(Modifier.weight(1f))
        } else {
            LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(12.dp), contentPadding = PaddingValues(bottom = 16.dp)) {
                items(projects, key = { it.id }) { project ->
                    Surface(shape = RoundedCornerShape(16.dp), modifier = Modifier.fillMaxWidth().clickable(enabled = !busy) { onOpen(project.id) }) {
                        Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                            Text(project.title, style = MaterialTheme.typography.h6, maxLines = 2)
                            Text(project.detail, color = MaterialTheme.colors.secondary)
                            Text(project.modified, style = MaterialTheme.typography.caption)
                        }
                    }
                }
            }
        }
        Row(Modifier.fillMaxWidth().padding(bottom = 12.dp), horizontalArrangement = Arrangement.SpaceBetween) {
            TextButton(onClick = onPairing, modifier = Modifier.heightIn(min = 48.dp)) { Text("Pair a computer") }
            TextButton(onClick = onDiagnostics, modifier = Modifier.heightIn(min = 48.dp)) { Text("Diagnostics") }
        }
    }
}
