package com.visualworkbench.android.editor

import android.graphics.Canvas
import android.graphics.RenderNode
import com.visualworkbench.android.precision.PrecisionDrawing
import com.visualworkbench.shared.Rect
import com.visualworkbench.shared.RenderItem
import com.visualworkbench.shared.Shape
import com.visualworkbench.shared.Transform

/** Whether [bounds] (a native conservative box, document space) could reach
 * into [visible] grown by [margin] on every side. Touching edges count, and a
 * box that cannot be judged is treated as reaching in. */
internal fun boundsMayTouch(bounds: Rect, visible: Rect, margin: Double): Boolean {
    if (!(margin >= 0.0) || !bounds.x.isFinite() || !bounds.y.isFinite() || !bounds.width.isFinite() || !bounds.height.isFinite()) return true
    return bounds.x <= visible.x + visible.width + margin && bounds.x + bounds.width >= visible.x - margin &&
        bounds.y <= visible.y + visible.height + margin && bounds.y + bounds.height >= visible.y - margin
}

/** The document-space box covered by a screen rectangle under [matrix]
 * (document to screen). Null when the matrix cannot be inverted. */
internal fun visibleDocumentRect(matrix: Transform, screen: Rect): Rect? {
    val determinant = matrix.a * matrix.d - matrix.b * matrix.c
    if (!determinant.isFinite() || kotlin.math.abs(determinant) < 1e-18) return null
    fun back(x: Double, y: Double): Pair<Double, Double> {
        val dx = x - matrix.e; val dy = y - matrix.f
        return (matrix.d * dx - matrix.c * dy) / determinant to (matrix.a * dy - matrix.b * dx) / determinant
    }
    val corners = listOf(back(screen.x, screen.y), back(screen.x + screen.width, screen.y),
        back(screen.x, screen.y + screen.height), back(screen.x + screen.width, screen.y + screen.height))
    val left = corners.minOf { it.first }; val top = corners.minOf { it.second }
    val right = corners.maxOf { it.first }; val bottom = corners.maxOf { it.second }
    if (!left.isFinite() || !top.isFinite() || !right.isFinite() || !bottom.isFinite()) return null
    return Rect(left, top, right - left, bottom - top)
}

/** What an ink frame has to know about the saved part of a scene, worked out
 * once for each kept picture: the order the layers are drawn in, the last
 * layer that has anything drawn, and how each layer is composited.
 *
 * [layers] are the document's layer ids in order, [saved] the saved objects in
 * drawing order; [hidden] objects are not drawn. */
internal class DryLayers(layers: List<String>, saved: List<RenderItem>, hidden: Set<String>) {
    // Layer order as the renderer builds it: the document's layers, then any other in first-use order.
    private val place = LinkedHashMap<String, Int>()
    // The whole layer is composited with the opacity and blend of its first drawn object.
    private val first = HashMap<String, RenderItem>()
    private var topDrawn = -1
    init {
        for (id in layers) place.getOrPut(id) { place.size }
        for (item in saved) {
            if (item.objectId in hidden) continue
            val at = place.getOrPut(item.layerId) { place.size }
            if (at > topDrawn) topDrawn = at
            if (item.layerId !in first) first[item.layerId] = item
        }
    }

    /** Whether the [wet] ink can be painted straight over a finished picture
     * of the saved scene and look exactly as when everything is drawn
     * together. That is so only when all of it sits in the layer drawn last
     * and that layer is composited plainly: full opacity, normal blend. A
     * highlighter (multiply) or a stroke beneath another layer needs the whole
     * scene, and gets it. */
    fun inkOnTop(wet: List<RenderItem>, hidden: Set<String>): Boolean {
        var layer: String? = null
        var lead: RenderItem? = null
        for (item in wet) {
            if (item.objectId in hidden) continue
            if (item.shape is Shape.Result) return false
            if (layer == null) { layer = item.layerId; lead = item } else if (item.layerId != layer) return false
        }
        if (layer == null || lead == null) return false
        // A layer nothing saved uses yet is drawn after every other one.
        val at = place[layer] ?: place.size
        if (topDrawn > at) return false
        val paint = first[layer] ?: lead
        if (paint.shape is Shape.Result) return false
        return paint.layerBlend != "multiply" && (paint.layerOpacity.coerceIn(0.0, 1.0) * 255).toInt() == 255
    }
}

internal fun wetInkOnTop(layers: List<String>, saved: List<RenderItem>, wet: List<RenderItem>, hidden: Set<String>): Boolean =
    DryLayers(layers, saved, hidden).inkOnTop(wet, hidden)

/** Draws an ink frame without redrawing the document: everything except the
 * stroke in progress is recorded once and kept by the renderer as a finished
 * layer, then each frame draws that layer, the wet ink on top, the overlays
 * and the magnifier panes. Owned by one render session and used only on its
 * drawing thread. When a scene does not qualify, [draw] draws nothing and
 * returns false, and the caller draws the whole scene as before. */
internal class WetInkLayer {
    private val node = RenderNode("vw-dry-scene")
    private var kept: Kept? = null

    /** What the kept picture was drawn from. Lists and bitmaps by identity. */
    private class Kept(val scene: CanvasScene, val width: Int, val height: Int, val layers: DryLayers) {
        fun sameAs(other: CanvasScene, w: Int, h: Int): Boolean {
            val s = scene
            if (w != width || h != height || s.document !== other.document || s.background !== other.background ||
                s.matrix != other.matrix || s.camera != other.camera || s.dark != other.dark ||
                s.smoothBackground != other.smoothBackground || s.hidden != other.hidden) return false
            val a = s.objects; val b = other.objects
            if (a === b) return true
            if (a.size != b.size) return false
            for (index in a.indices) if (a[index] !== b[index]) return false
            return true
        }
    }

    fun draw(canvas: Canvas, scene: CanvasScene): Boolean {
        if (!canvas.isHardwareAccelerated || scene.giantMode || scene.giantFrame != null || scene.documentFrame != null ||
            scene.documentDraft != null || !scene.resultDisplayReady || scene.aiResults.isNotEmpty() || scene.transformed.isNotEmpty() ||
            scene.provisional.isEmpty()) return false
        val width = scene.camera.viewportWidth.toInt(); val height = scene.camera.viewportHeight.toInt()
        if (width <= 0 || height <= 0) return false
        val current = kept?.takeIf { it.sameAs(scene, width, height) }
        val layers = current?.layers ?: DryLayers(scene.document.layers.map { it.id }, scene.objects.map { it.item }, scene.hidden)
        if (!layers.inkOnTop(scene.provisional.map { it.item }, scene.hidden)) return false
        if (current == null) {
            node.setPosition(0, 0, width, height)
            val recording = node.beginRecording(width, height)
            try { EditorDrawing.drawDry(recording, scene) } finally { node.endRecording() }
            // Ask the renderer to keep the result as a finished layer instead of replaying it.
            node.setUseCompositingLayer(true, null)
            kept = Kept(scene, width, height, layers)
        }
        // The surround first, in case the frame is larger than the kept layer.
        canvas.drawColor(EditorDrawing.surroundArgb(scene.dark))
        canvas.drawRenderNode(node)
        EditorDrawing.drawWetOnTop(canvas, scene)
        EditorDrawing.drawOverlays(canvas, scene)
        PrecisionDrawing.drawPanes(canvas, scene, onlyVisible = true)
        return true
    }

    /** Forget the kept picture; the next ink frame records it again. */
    fun discard() { kept = null; node.discardDisplayList() }
}
