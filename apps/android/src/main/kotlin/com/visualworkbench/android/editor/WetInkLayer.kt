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

/** Whether two saved-object lists are the same objects in the same order. */
private fun sameObjects(a: List<DrawObject>, b: List<DrawObject>): Boolean {
    if (a === b) return true
    if (a.size != b.size) return false
    for (index in a.indices) if (a[index] !== b[index]) return false
    return true
}

/** Draws an ink frame without redrawing the document: everything except the
 * stroke in progress is recorded once and kept by the renderer as a finished
 * layer, then each frame draws that layer, the wet ink on top, the overlays
 * and the magnifier panes. When a scene does not qualify, [draw] draws nothing
 * and returns false, and the caller draws the whole scene as before.
 *
 * Owned by one render session. [draw] and [forget] run on its drawing thread;
 * [discard] runs on the main thread only after that thread has stopped drawing
 * for the session, which is why the two references are volatile. The picture
 * is kept only while ink is wet: [forget] drops it at every saved frame, so
 * between strokes this holds no bitmap or document, and the renderer is free
 * to release the finished layer once the front buffer no longer shows it. */
internal class WetInkLayer {
    private val node = RenderNode("vw-dry-scene")
    @Volatile private var kept: Kept? = null
    @Volatile private var judged: Judged? = null

    /** The layer order of one saved scene, worked out once whether or not a picture is kept for it. */
    private class Judged(val document: Any, val objects: List<DrawObject>, val hidden: Set<String>, val layers: DryLayers) {
        fun sameAs(scene: CanvasScene): Boolean =
            document === scene.document && (hidden === scene.hidden || hidden == scene.hidden) && sameObjects(objects, scene.objects)
    }

    /** What the kept picture was drawn from. Lists and bitmaps by identity. */
    private class Kept(val scene: CanvasScene, val width: Int, val height: Int) {
        fun sameAs(other: CanvasScene, w: Int, h: Int): Boolean {
            val s = scene
            return w == width && h == height && s.document === other.document && s.background === other.background &&
                s.matrix == other.matrix && s.camera == other.camera && s.dark == other.dark &&
                s.smoothBackground == other.smoothBackground && s.hidden == other.hidden && sameObjects(s.objects, other.objects)
        }
    }

    fun draw(canvas: Canvas, scene: CanvasScene): Boolean {
        if (!canvas.isHardwareAccelerated || scene.giantMode || scene.giantFrame != null || scene.documentFrame != null ||
            scene.documentDraft != null || !scene.resultDisplayReady || scene.aiResults.isNotEmpty() || scene.transformed.isNotEmpty() ||
            scene.provisional.isEmpty()) { forget(); return false }
        val width = scene.camera.viewportWidth.toInt(); val height = scene.camera.viewportHeight.toInt()
        if (width <= 0 || height <= 0) { forget(); return false }
        // The layer order is worked out once for a saved scene, also when the
        // answer is no: a highlighter stroke or a stroke under another layer
        // costs one small check per frame, not a pass over every object.
        val layers = (judged?.takeIf { it.sameAs(scene) }
            ?: Judged(scene.document, scene.objects, scene.hidden,
                DryLayers(scene.document.layers.map { it.id }, scene.objects.map { it.item }, scene.hidden)).also { judged = it }).layers
        if (!layers.inkOnTop(scene.provisional.map { it.item }, scene.hidden)) { forgetPicture(); return false }
        if (kept?.sameAs(scene, width, height) != true) {
            // A picture of an earlier document or view is dropped before the new one is recorded.
            forgetPicture()
            node.setPosition(0, 0, width, height)
            val recording = node.beginRecording(width, height)
            try { EditorDrawing.drawDry(recording, scene) } finally { node.endRecording() }
            // Ask the renderer to keep the result as a finished layer instead of replaying it.
            node.setUseCompositingLayer(true, null)
            kept = Kept(scene, width, height)
        }
        // The surround first, in case the frame is larger than the kept layer.
        canvas.drawColor(EditorDrawing.surroundArgb(scene.dark))
        canvas.drawRenderNode(node)
        EditorDrawing.drawWetOnTop(canvas, scene)
        EditorDrawing.drawOverlays(canvas, scene)
        PrecisionDrawing.drawPanes(canvas, scene, onlyVisible = true)
        return true
    }

    private fun forgetPicture() { if (kept != null) { kept = null; node.discardDisplayList() } }

    /** Drawing thread: a saved frame was drawn, or a frame this layer cannot
     * draw. Nothing is kept until the next ink frame records it again. */
    fun forget() { forgetPicture(); judged = null }

    /** Main thread, after the drawing thread has stopped for this session. */
    fun discard() { kept = null; judged = null; node.discardDisplayList() }
}
