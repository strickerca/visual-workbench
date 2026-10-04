package com.visualworkbench.android.editor

import android.graphics.BlendMode
import android.graphics.Canvas
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Region
import com.visualworkbench.shared.RenderItem
import com.visualworkbench.shared.Shape
import com.visualworkbench.shared.Transform
import com.visualworkbench.shared.Point

/** Object selection uses the exact transformed image rectangle. Alpha does
 * not remove a Result's editable bounds; its pixel erasing is a mask action. */
internal fun aiResultHit(scene: CanvasScene, item: RenderItem, screen: Point, radius: Float): Boolean {
    val pixels = scene.aiResults[item.objectId] ?: return false
    if (!scene.resultDisplayReady || !radius.isFinite() || radius < 0) return false
    val path = Path().apply { addRect(0f,0f,pixels.width.toFloat(),pixels.height.toFloat(),Path.Direction.CW) }
    path.transform(EditorDrawing.matrix(EditorDrawing.compose(scene.matrix,scene.transformed[item.objectId]?:item.transform)))
    val expanded = Path()
    Paint().apply { style=Paint.Style.FILL_AND_STROKE;strokeWidth=radius*2;strokeJoin=Paint.Join.ROUND }.getFillPath(path,expanded)
    val region = Region()
    region.setPath(expanded,Region(0,0,scene.camera.viewportWidth.toInt(),scene.camera.viewportHeight.toInt()))
    return screen.x.isFinite() && screen.y.isFinite() && region.contains(screen.x.toInt(),screen.y.toInt())
}

/** Render in the transparent document stack, with the page outside that stack.
 * DstOut + Plus implements premultiplied replacement interpolation, preserving
 * exterior alpha. Never wrap this Result in an ordinary SrcOver layer. */
internal fun drawAiResult(canvas: Canvas, item: RenderItem, pixels: AiResultImage<AiBitmap>,
    documentToScreen: Transform, pose: Transform = item.transform) {
    val shape = item.shape as? Shape.Result ?: return
    require(shape.resultId == pixels.resultId && shape.assetId == pixels.compositeAssetId && item.objectId == pixels.objectId)
    require(item.layerBlend == "normal" && item.layerOpacity.isFinite() && item.layerOpacity in 0.0..1.0)
    val alpha = (item.layerOpacity * 255.0 + .5).toInt().coerceIn(0, 255)
    if (alpha == 0) return
    val view = Transform(
        documentToScreen.a * pose.a + documentToScreen.c * pose.b,
        documentToScreen.b * pose.a + documentToScreen.d * pose.b,
        documentToScreen.a * pose.c + documentToScreen.c * pose.d,
        documentToScreen.b * pose.c + documentToScreen.d * pose.d,
        documentToScreen.a * pose.e + documentToScreen.c * pose.f + documentToScreen.e,
        documentToScreen.b * pose.e + documentToScreen.d * pose.f + documentToScreen.f)
    val transform = Matrix().apply { setValues(floatArrayOf(view.a.toFloat(), view.c.toFloat(), view.e.toFloat(),
        view.b.toFloat(), view.d.toFloat(), view.f.toFloat(), 0f, 0f, 1f)) }
    val saved = canvas.save()
    try {
        canvas.concat(transform)
        val remove = Paint().apply { color = android.graphics.Color.BLACK; this.alpha = alpha; blendMode = BlendMode.DST_OUT; isAntiAlias = false }
        val add = Paint().apply { this.alpha = alpha; blendMode = BlendMode.PLUS; isFilterBitmap = false; isAntiAlias = false }
        for (tile in pixels.tiles) {
            val x = tile.region.x.toFloat(); val y = tile.region.y.toFloat()
            canvas.drawRect(x, y, x + tile.region.width.toFloat(), y + tile.region.height.toFloat(), remove)
            canvas.drawBitmap(tile.image.image, x, y, add)
        }
    } finally { canvas.restoreToCount(saved) }
}
