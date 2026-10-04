package com.visualworkbench.android.editor

import android.graphics.Bitmap
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.unit.IntOffset
import com.visualworkbench.shared.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

private data class SelectionPixels(val source: SelectionPreview, val images: List<Pair<SelectionRegion, ImageBitmap>>)
internal fun selectionArgb(coverage: ByteArray): IntArray = IntArray(coverage.size) { index ->
    val alpha = ((coverage[index].toInt() and 255) * 110 + 127) / 255
    (alpha shl 24) or 0x00d15bea
}
private fun Transform.selectionMatrix(): Matrix = Matrix(floatArrayOf(a.toFloat(), b.toFloat(), 0f, 0f,
    c.toFloat(), d.toFloat(), 0f, 0f, 0f, 0f, 1f, 0f, e.toFloat(), f.toFloat(), 0f, 1f))

/** Separate Compose overlay: it never recycles a Bitmap still referenced by
 * RenderThread/front-buffer work. Immutable bounded images follow ImageBitmap
 * lifetime; no camera frames or image bytes are persisted. */
@Composable
internal fun SelectionOverlay(controller: SelectionController, camera: Camera, revision: ProjectInfo?, documentId: String?, modifier: Modifier = Modifier) {
    val state by controller.state.collectAsState()
    val preview = state.preview?.takeIf { revision != null && documentId != null && it.binding.matches(revision, documentId) }
    var pixels by remember { mutableStateOf<SelectionPixels?>(null) }
    LaunchedEffect(preview) {
        pixels = null
        if (preview == null) return@LaunchedEffect
        val images = withContext(Dispatchers.Default) {
            preview.tiles.map { tile ->
                currentCoroutineContext().ensureActive()
                val width = tile.region.width.toInt(); val height = tile.region.height.toInt()
                require(width in 1..256 && height in 1..256 && tile.coverage.size == width * height)
                tile.region to Bitmap.createBitmap(selectionArgb(tile.coverage), width, height, Bitmap.Config.ARGB_8888).asImageBitmap()
            }
        }
        pixels = SelectionPixels(preview, images)
    }
    Canvas(modifier.fillMaxSize()) {
        if (!state.active || revision == null || documentId == null || state.document?.binding?.matches(revision, documentId) != true) return@Canvas
        val source = state.document ?: return@Canvas
        withTransform({ transform(controller.core.cameraMatrix(camera).selectionMatrix()) }) {
            clipRect(0f, 0f, source.width.toFloat(), source.height.toFloat()) {
                pixels?.takeIf { it.source === preview }?.images?.forEach { (region, image) ->
                    drawImage(image, dstOffset = IntOffset(region.x.toInt(), region.y.toInt()), filterQuality = FilterQuality.None)
                }
                state.draft?.let { draft ->
                    val first = draft.points.firstOrNull() ?: return@let; val last = draft.points.last()
                    val color = Color(0xffedb6ff); val line = Stroke((1.5 / camera.scale).toFloat())
                    if (draft.tool == SelectionTool.Rectangle) drawRect(color,
                        Offset(minOf(first.x, last.x).toFloat(), minOf(first.y, last.y).toFloat()),
                        Size(kotlin.math.abs(last.x - first.x).toFloat(), kotlin.math.abs(last.y - first.y).toFloat()), style = line)
                    else {
                        val path = Path().apply { moveTo(first.x.toFloat(), first.y.toFloat()); draft.points.drop(1).forEach { lineTo(it.x.toFloat(), it.y.toFloat()) }; if (draft.tool == SelectionTool.Lasso) close() }
                        drawPath(path, color, style = line)
                        if (draft.tool == SelectionTool.Paint || draft.tool == SelectionTool.MaskEraser)
                            drawCircle(color, draft.radius.toFloat(), Offset(last.x.toFloat(), last.y.toFloat()), style = line)
                    }
                }
            }
        }
    }
}
