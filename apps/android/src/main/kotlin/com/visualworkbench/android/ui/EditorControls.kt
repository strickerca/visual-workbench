package com.visualworkbench.android.ui

import android.graphics.Paint
import android.graphics.Path
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path as ComposePath
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.visualworkbench.android.editor.*

@Composable
internal fun ToolRail(tool: EditorTool, onTool: (EditorTool) -> Unit, onMove: (Float) -> Unit, modifier: Modifier = Modifier) {
    val move by rememberUpdatedState(onMove)
    Surface(modifier.width(68.dp), shape = RoundedCornerShape(18.dp), elevation = 6.dp) {
        Column(Modifier.padding(6.dp)) {
            Box(Modifier.fillMaxWidth().height(48.dp).semantics { contentDescription = "Move tool rail vertically" }
                .pointerInput(Unit) { detectDragGestures { change, drag -> change.consume(); move(drag.y) } }, contentAlignment = Alignment.Center) {
                Text("⋮⋮", color = MaterialTheme.colors.onSurface, fontSize = 22.sp)
            }
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                EditorTool.entries.forEach { candidate ->
                    val selected = tool == candidate
                    Box(Modifier.fillMaxWidth().height(52.dp).background(
                        if (selected) MaterialTheme.colors.primary else MaterialTheme.colors.surface, RoundedCornerShape(12.dp))
                        .semantics { contentDescription = candidate.label; role = Role.RadioButton; this.selected = selected }
                        .clickable { onTool(candidate) }, contentAlignment = Alignment.Center) {
                        Text(candidate.shortLabel, color = if (selected) MaterialTheme.colors.onPrimary else MaterialTheme.colors.onSurface,
                            fontSize = 12.sp, fontWeight = if (selected) FontWeight.Bold else FontWeight.Medium)
                    }
                }
            }
        }
    }
}

@Composable
internal fun QuickControls(brush: BrushPreference, onBrush: (BrushPreference) -> Unit, onDetails: () -> Unit,
                          canUndo: Boolean, canRedo: Boolean, onUndo: () -> Unit, onRedo: () -> Unit) {
    Surface(elevation = 8.dp) {
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = onUndo, enabled = canUndo, modifier = Modifier.heightIn(min = 48.dp)) { Text("Undo") }
            OutlinedButton(onClick = onRedo, enabled = canRedo, modifier = Modifier.heightIn(min = 48.dp)) { Text("Redo") }
            listOf(0xff51c7c2L, 0xffe76073L, 0xffffc857L, 0xff182d39L, 0xfffafafaL).forEachIndexed { index, value ->
                val names = listOf("Teal", "Rose", "Amber", "Ink", "White")
                val chosen = (brush.colorArgb and 0xffffff) == (value and 0xffffff)
                Box(Modifier.size(48.dp).semantics { contentDescription = "${names[index]} stroke color"; selected = chosen }
                    .clickable { onBrush(brush.copy(colorArgb = (brush.colorArgb and 0xff000000) or (value and 0xffffff))) }, contentAlignment = Alignment.Center) {
                    Box(Modifier.size(28.dp).background(Color(value), CircleShape)
                        .border(if (chosen) 3.dp else 1.dp, MaterialTheme.colors.onSurface, CircleShape))
                }
            }
            OutlinedButton(onClick = onDetails, modifier = Modifier.heightIn(min = 48.dp)) { Text("${brush.width.toInt()} px · Brush") }
        }
    }
}

@Composable
internal fun BrushDialog(tool: EditorTool, brush: BrushPreference, preview: Path?, onChange: (BrushPreference) -> Unit, onDismiss: () -> Unit) {
    AlertDialog(onDismissRequest = onDismiss, title = { Text("${tool.label} settings") }, text = {
        Column(Modifier.heightIn(max = 560.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Width · ${"%.1f".format(brush.width)} document pixels")
            Slider(value = brush.width.toFloat(), onValueChange = { onChange(brush.copy(width = it.toDouble())) }, valueRange = .25f..256f,
                modifier = Modifier.heightIn(min = 48.dp).semantics { contentDescription = "Brush width" })
            if (tool in listOf(EditorTool.Pen, EditorTool.Highlighter, EditorTool.Marker)) {
            Text("Stabilization · ${(brush.stabilization * 100).toInt()}%")
            Slider(value = brush.stabilization, onValueChange = { onChange(brush.copy(stabilization = it)) },
                modifier = Modifier.heightIn(min = 48.dp).semantics { contentDescription = "Stroke stabilization" })
            Text("Pressure curve", fontWeight = FontWeight.Bold)
            Text("Input pressure → stroke width. The endpoints stay at zero and one; both control points remain monotonic.", style = MaterialTheme.typography.caption)
            val foreground = MaterialTheme.colors.primary
            val grid = MaterialTheme.colors.onSurface.copy(alpha = .35f)
            Canvas(Modifier.fillMaxWidth().height(128.dp).semantics { contentDescription = "Pressure curve from zero to full pressure" }) {
                val pad = 8.dp.toPx(); val w = size.width - 2 * pad; val h = size.height - 2 * pad
                fun point(x: Float, y: Float) = Offset(pad + w * x, pad + h * (1 - y))
                drawLine(grid, point(0f, 0f), point(1f, 1f), 1.dp.toPx())
                val path = ComposePath().apply { moveTo(pad, pad + h); cubicTo(point(brush.pressure.x1, brush.pressure.y1).x,
                    point(brush.pressure.x1, brush.pressure.y1).y, point(brush.pressure.x2, brush.pressure.y2).x,
                    point(brush.pressure.x2, brush.pressure.y2).y, pad + w, pad) }
                drawPath(path, foreground, style = Stroke(3.dp.toPx()))
                drawCircle(foreground, 5.dp.toPx(), point(brush.pressure.x1, brush.pressure.y1))
                drawCircle(foreground, 5.dp.toPx(), point(brush.pressure.x2, brush.pressure.y2))
            }
            listOf("First point · pressure", "First point · width", "Second point · pressure", "Second point · width").forEachIndexed { index, label ->
                Text(label, style = MaterialTheme.typography.caption)
                val values = listOf(brush.pressure.x1, brush.pressure.y1, brush.pressure.x2, brush.pressure.y2)
                Slider(value = values[index], onValueChange = { onChange(brush.copy(pressure = brush.pressure.withControl(index, it))) },
                    modifier = Modifier.heightIn(min = 48.dp).semantics { contentDescription = label })
            }
            Text("Core stroke preview", style = MaterialTheme.typography.caption)
            Canvas(Modifier.fillMaxWidth().height(72.dp).background(Color.White, RoundedCornerShape(8.dp))) {
                preview?.let { path -> drawIntoCanvas { canvas ->
                    val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = brush.colorArgb.toInt(); style = Paint.Style.FILL }
                    canvas.nativeCanvas.save()
                    canvas.nativeCanvas.scale(size.width / 320f, size.height / 72f)
                    canvas.nativeCanvas.drawPath(path, paint)
                    canvas.nativeCanvas.restore()
                } }
            }
            }
        }
    }, confirmButton = { TextButton(onClick = onDismiss, modifier = Modifier.heightIn(min = 48.dp)) { Text("Done") } })
}

@Composable
internal fun SettingsScreen(settings: EditorPreferences, onChange: (EditorPreferences) -> Unit, onBack: () -> Unit) {
    Column(Modifier.fillMaxSize().padding(20.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        TextButton(onClick = onBack, modifier = Modifier.heightIn(min = 48.dp)) { Text("← Back") }
        Text("Your workspace", style = MaterialTheme.typography.h4)
        Text("Canvas & controls", style = MaterialTheme.typography.subtitle1, color = MaterialTheme.colors.primary)
        SettingSwitch("Draw with finger", "Off by default. Fingers pan and zoom; a pen always uses the active tool.", settings.drawWithFinger) { onChange(settings.copy(drawWithFinger = it)) }
        SettingSwitch("Left-handed layout", "Mirror the tool rail to the right edge.", settings.leftHanded) { onChange(settings.copy(leftHanded = it)) }
        SettingSwitch("Dark theme", "A quieter frame around your image. Document colors stay unchanged.", settings.darkTheme) { onChange(settings.copy(darkTheme = it)) }
        SettingSwitch("Reduce motion", "Keep navigation and controls still. Android's animation preference is also respected.", settings.reducedMotion) { onChange(settings.copy(reducedMotion = it)) }
        Text("Tool rail position")
        Slider(settings.railOffset, { onChange(settings.copy(railOffset = it)) }, modifier = Modifier.heightIn(min = 48.dp).semantics { contentDescription = "Tool rail vertical position" })
        Text("You can also drag the handle above the tools. Brush pressure curves and stabilization are saved separately for each brush.", style = MaterialTheme.typography.body2)
        Text("Gestures", style = MaterialTheme.typography.h6)
        Text("One finger: pan\nTwo fingers: zoom and rotate\nTwo-finger tap: undo\nThree-finger tap: redo\nHold one finger: canvas menu")
        Text("Generic stylus profile", style = MaterialTheme.typography.h6)
        Text("Pressure, tilt, hover, eraser and buttons use standard Android input. Samsung actions are optional and currently disabled.")
    }
}

@Composable
private fun SettingSwitch(title: String, description: String, checked: Boolean, change: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min = 72.dp).clickable { change(!checked) }.padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) { Text(title, fontWeight = FontWeight.Medium); Text(description, style = MaterialTheme.typography.body2) }
        Switch(checked = checked, onCheckedChange = change, modifier = Modifier.sizeIn(minWidth = 48.dp, minHeight = 48.dp).semantics { contentDescription = title })
    }
}
