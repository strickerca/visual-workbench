package com.visualworkbench.android.remote

import androidx.compose.foundation.Canvas
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import com.visualworkbench.shared.RemoteGhostInk
import com.visualworkbench.shared.RemoteTargetBinding
import com.visualworkbench.shared.VisibleGhost
import kotlinx.coroutines.isActive

/** Coordinates already map to this overlay's local pixels. This component never
 * maps touch coordinates, supplies input acknowledgments or freezes video. */
@Composable
fun RemoteGhostOverlay(
    ink: RemoteGhostInk, binding: RemoteTargetBinding?, active: Boolean,
    modifier: Modifier = Modifier, color: Color = Color(0xff65d8ff),
) {
    var visible by remember(ink) { mutableStateOf<List<VisibleGhost>>(emptyList()) }
    DisposableEffect(ink, binding, active) {
        if (!active) { if (binding != null) ink.retire(binding); visible = emptyList() }
        onDispose { if (binding != null) ink.retire(binding); visible = emptyList() }
    }
    LaunchedEffect(ink, binding, active) {
        if (!active || binding == null) { visible = emptyList(); return@LaunchedEffect }
        while (isActive) {
            // Choreographer's supplied frame time is an estimate; expiry uses
            // the same System.nanoTime clock as admission/callback timestamps.
            withFrameNanos { visible = ink.snapshot(System.nanoTime()) }
        }
    }
    Canvas(modifier) {
        for (ghost in visible) {
            if (!active || binding == null || ghost.segment.binding != binding) continue
            val points = ghost.segment.points
            for (index in points.indices) {
                val point = points[index]
                val opacity = ghost.alpha * if (point.predicted) 0.45f else 1f
                val width = 1.5f + 3.5f * point.pressure
                if (index == 0) drawCircle(color.copy(alpha = opacity), width / 2,
                    Offset(point.x, point.y))
                else {
                    val previous = points[index - 1]
                    drawLine(color.copy(alpha = opacity), Offset(previous.x, previous.y),
                        Offset(point.x, point.y), width)
                }
            }
        }
    }
}
