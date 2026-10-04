package com.visualworkbench.desktop

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
import org.jetbrains.skia.Image
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import javax.imageio.ImageIO

private data class SelectionPixels(val source: SelectionPreview, val images: List<Pair<SelectionRegion, ImageBitmap>>)

/** Display-only magenta alpha. Source coverage is never altered or used for
 * export. Native tile results are exact D-space pixels; no smoothing/resizing. */
internal fun selectionArgb(coverage: ByteArray): IntArray = IntArray(coverage.size) { i ->
    val alpha = ((coverage[i].toInt() and 255) * 110 + 127) / 255
    (alpha shl 24) or 0x00d15bea
}

@Composable
internal fun SelectionOverlay(controller: SelectionController, camera: Camera, revision: ProjectInfo?, documentId: String?, modifier: Modifier = Modifier) {
    val state by controller.state.collectAsState()
    val preview = state.preview?.takeIf { revision != null && documentId != null && it.binding.matches(revision, documentId) }
    var prepared by remember { mutableStateOf<SelectionPixels?>(null) }
    LaunchedEffect(preview) {
        prepared = null
        if (preview == null) return@LaunchedEffect
        val images = withContext(Dispatchers.Default) {
            preview.tiles.map { tile ->
                currentCoroutineContext().ensureActive()
                val w = tile.region.width.toInt(); val h = tile.region.height.toInt()
                require(w in 1..256 && h in 1..256 && tile.coverage.size == w * h)
                val image = BufferedImage(w, h, BufferedImage.TYPE_INT_ARGB)
                image.setRGB(0, 0, w, h, selectionArgb(tile.coverage), 0, w)
                val bytes = ByteArrayOutputStream().use { check(ImageIO.write(image, "png", it)); it.toByteArray() }
                tile.region to Image.makeFromEncoded(bytes).toComposeImageBitmap()
            }
        }
        prepared = SelectionPixels(preview, images)
    }
    Canvas(modifier.fillMaxSize()) {
        if (!state.active || revision == null || documentId == null || state.document?.binding?.matches(revision, documentId) != true) return@Canvas
        val source = state.document ?: return@Canvas
        withTransform({ transform(controller.core.cameraMatrix(camera).matrix()) }) {
            clipRect(0f, 0f, source.width.toFloat(), source.height.toFloat()) {
                prepared?.takeIf { it.source === preview }?.images?.forEach { (region, image) ->
                    drawImage(image, dstOffset = IntOffset(region.x.toInt(), region.y.toInt()), filterQuality = FilterQuality.None)
                }
                state.draft?.let { draft ->
                    val first = draft.points.firstOrNull() ?: return@let
                    val last = draft.points.last()
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
