@file:OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
package com.visualworkbench.android.editor

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.*
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.IntOffset
import com.visualworkbench.shared.*
import kotlinx.coroutines.delay

internal fun aiMatrix(t: Transform): Matrix = Matrix(floatArrayOf(t.a.toFloat(), t.b.toFloat(), 0f, 0f,
    t.c.toFloat(), t.d.toFloat(), 0f, 0f, 0f, 0f, 1f, 0f, t.e.toFloat(), t.f.toFloat(), 0f, 1f))

/** Both apps show the exact regions returned by native comparison. Touch pans;
 * a real pen/primary mouse contact uses the explicitly selected acceptance tool.
 * Predicted points never enter the acceptance mask. */
@Composable
internal fun AiCompareCanvas(controller: AiEditorController<AiBitmap>, core: WorkbenchCore,
    state: AiEditorState<AiBitmap>, modifier: Modifier = Modifier) {
    val latest by rememberUpdatedState(state)
    LaunchedEffect(state.open, state.view) {
        if (state.open && state.view == AiView.Blink) while (true) { delay(500); controller.blink() }
    }
    Column(modifier) {
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            TextButton({ controller.fit() }) { Text("Fit") }
            TextButton({ controller.zoom(.8) }) { Text("−") }
            TextButton({ controller.zoom(1.25) }) { Text("+") }
            Text("${(state.camera.scale * 100).toInt()}% · full-resolution regions", Modifier.padding(top = 12.dp), style = MaterialTheme.typography.caption)
        }
        Canvas(Modifier.fillMaxWidth().weight(1f).clipToBounds()
            .onSizeChanged { controller.viewport(it.width.toDouble(), it.height.toDouble()) }
            .pointerInput(controller) {
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    val canDraw = down.type != PointerType.Touch && !currentEvent.buttons.isSecondaryPressed && !currentEvent.buttons.isTertiaryPressed
                    var drawing = canDraw && controller.contactDown(Point(down.position.x.toDouble(), down.position.y.toDouble()))
                    var completed = false
                    var previous = down.position
                    down.consume()
                    try {
                        while (true) {
                            val event = awaitPointerEvent()
                            val change = event.changes.firstOrNull { it.id == down.id } ?: break
                            if (event.changes.count { it.pressed } > 1 && drawing) { controller.cancelContact(); drawing = false }
                            if (!change.pressed) {
                                if (drawing) {
                                    controller.contactMove(listOf(Point(change.position.x.toDouble(), change.position.y.toDouble())))
                                    controller.contactUp()
                                }
                                change.consume(); completed = true; break
                            }
                            if (drawing) {
                                val actual = change.historical.take(AI_CONTACT_POINTS + 1).map { Point(it.position.x.toDouble(), it.position.y.toDouble()) } +
                                    Point(change.position.x.toDouble(), change.position.y.toDouble())
                                controller.contactMove(actual)
                            } else controller.pan((change.position.x - previous.x).toDouble(), (change.position.y - previous.y).toDouble())
                            previous = change.position; change.consume()
                        }
                    } finally { if (!completed) controller.cancelContact() }
                }
            }) {
            drawRect(Color(0xff18222e))
            val current = latest
            val frame = current.frame ?: return@Canvas
            if (frame.camera != current.camera || frame.candidateId != current.candidate?.candidateId) return@Canvas
            withTransform({ transform(aiMatrix(core.cameraMatrix(current.camera))) }) {
                drawRect(Color.White, size = Size(frame.width.toFloat(), frame.height.toFloat()))
                for (tile in frame.tiles) drawImage(tile.image.display,
                    dstOffset = IntOffset(tile.region.x.toInt(), tile.region.y.toInt()), filterQuality = FilterQuality.None)
                if (current.contact.isNotEmpty()) {
                    val points = current.contact
                    val path = Path().apply {
                        moveTo(points.first().x.toFloat(), points.first().y.toFloat())
                        for (p in points.drop(1)) lineTo(p.x.toFloat(), p.y.toFloat())
                    }
                    fun preview() {
                        if (points.size == 1) drawCircle(Color(0x777fe0c5), current.radius.toFloat(), Offset(points[0].x.toFloat(), points[0].y.toFloat()))
                        else drawPath(path, Color(0x777fe0c5), style = Stroke((2 * current.radius).toFloat(), cap = StrokeCap.Round, join = StrokeJoin.Round))
                    }
                    preview()
                    if (current.view == AiView.Split) withTransform({ translate(checkNotNull(current.candidate).width.toFloat(), 0f) }) { preview() }
                }
            }
        }
        if (state.loadingPixels) LinearProgressIndicator(Modifier.fillMaxWidth())
        Text(if (state.brush) "Pen or primary mouse: accept pixels. Finger: pan. One candidate-mask change per contact."
            else "Drag to pan. Choose a comparison or enable the acceptance brush.", style = MaterialTheme.typography.caption)
    }
}
