package com.visualworkbench.desktop

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.unit.IntOffset
import com.visualworkbench.shared.RenderItem
import com.visualworkbench.shared.Shape
import com.visualworkbench.shared.Transform

/** Call inside the transparent document stack, before it is shown over the
 * white page. A full-opacity materialized Result REPLACES samples, including
 * transparency. Opacity interpolates both premultiplied color and alpha. This
 * is not an ordinary SrcOver image/layer, which would increase exterior alpha.
 * The caller has admitted a dedicated normal-blend, one-Result layer. */
internal fun DrawScope.drawAiResult(item: RenderItem, pixels: AiResultImage<AiBitmap>, pose: Transform = item.transform) {
    val shape = item.shape as? Shape.Result ?: return
    require(shape.resultId == pixels.resultId && shape.assetId == pixels.compositeAssetId && item.objectId == pixels.objectId)
    require(item.layerBlend == "normal" && item.layerOpacity.isFinite() && item.layerOpacity in 0.0..1.0)
    val opacity = item.layerOpacity.toFloat()
    if (opacity == 0f) return
    withTransform({ transform(aiMatrix(pose)) }) {
        for (tile in pixels.tiles) {
            val at = Offset(tile.region.x.toFloat(), tile.region.y.toFloat())
            val extent = Size(tile.region.width.toFloat(), tile.region.height.toFloat())
            // Axis-aligned tile edges within one transformed image must use the
            // same non-antialiased coverage for removal and addition.
            val remove = Paint().apply { color = Color.Black; alpha = opacity; blendMode = BlendMode.DstOut; isAntiAlias = false }
            drawContext.canvas.drawRect(androidx.compose.ui.geometry.Rect(at, extent), remove)
            drawImage(tile.image.display, dstOffset = IntOffset(tile.region.x.toInt(), tile.region.y.toInt()), alpha = opacity, blendMode = BlendMode.Plus, filterQuality = FilterQuality.None)
        }
    }
}
