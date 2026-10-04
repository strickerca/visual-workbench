package com.visualworkbench.desktop

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.graphics.drawscope.withTransform
import com.visualworkbench.shared.*

@Composable
internal fun EraserOverlay(controller: EraserController, camera: Camera, revision: ProjectInfo?, documentId: String?, modifier: Modifier = Modifier) {
    val state by controller.state.collectAsState()
    val draft = state.draft?.takeIf { revision != null && it.binding.projectId == revision.projectId &&
        it.binding.hostSeq == revision.hostSeq && it.binding.stateHash == revision.stateHash &&
        it.binding.documentId == documentId && it.camera == camera }
    Canvas(modifier.fillMaxSize()) {
        val current = draft ?: return@Canvas
        withTransform({ transform(controller.core.cameraMatrix(camera).matrix()) }) {
            clipRect(0f, 0f, current.width.toFloat(), current.height.toFloat()) {
                val color = Color(0x55ff846b); val first = current.points.first()
                if (current.points.size == 1) drawCircle(color, current.radius.toFloat(), Offset(first.x.toFloat(), first.y.toFloat()))
                else {
                    val path = Path().apply { moveTo(first.x.toFloat(), first.y.toFloat()); current.points.drop(1).forEach { lineTo(it.x.toFloat(), it.y.toFloat()) } }
                    drawPath(path, color, style = Stroke((2 * current.radius).toFloat(), cap = StrokeCap.Round, join = StrokeJoin.Round))
                }
            }
        }
    }
}
