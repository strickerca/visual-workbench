package com.visualworkbench.android.editor

import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Region
import com.visualworkbench.shared.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlin.math.max

/** Core-shaped label, document-space geometry, same independent circle/box
 * fill+stroke passes as vw-raster. No Android font fallback. */
internal data class MarkerPaint(val circle: Path, val box: Path?, val label: Path) {
    /** Union the separately painted components, rather than treating overlapping
     * circle/box contours as one winding-sensitive shape. Outline expansion
     * uses the same coordinate space as draw(), before view-space hit slop. */
    fun hit(scene: CanvasScene, obj: DrawObject, screen: Point, radius: Float): Boolean {
        if (!radius.isFinite() || radius < 0f || obj.item.layerOpacity <= 0.0) return false
        val style = obj.item.style
        val objectMatrix = EditorDrawing.compose(scene.transformed[obj.item.objectId] ?: obj.item.transform,
            Transform(e = obj.origin.x, f = obj.origin.y))
        val fullMatrix = EditorDrawing.compose(scene.matrix, objectMatrix)
        val painted = Path()
        fun union(path: Path, mapping: Transform): Boolean {
            val view = Path(path).apply { transform(EditorDrawing.matrix(mapping)) }
            return painted.op(view, Path.Op.UNION)
        }
        fun outlined(path: Path): Boolean {
            if (style.fill?.let { (it and 0xffu) != 0u } == true && !union(path, fullMatrix)) return false
            if (style.width <= 0.0 || (style.rgba and 0xffu) == 0u) return true
            val constant = style.screenConstantWidth
            val line = if (constant) Path(path).apply { transform(EditorDrawing.matrix(objectMatrix)) } else path
            val expanded = Path()
            Paint().apply {
                this.style = Paint.Style.STROKE
                strokeWidth = (if (constant) style.width / scene.camera.scale else style.width).toFloat()
                strokeJoin = Paint.Join.ROUND; strokeCap = Paint.Cap.ROUND
            }.getFillPath(line, expanded)
            return union(expanded, if (constant) scene.matrix else fullMatrix)
        }
        if (!outlined(circle) || (box != null && !outlined(box))) return false
        if ((style.rgba and 0xffu) != 0u && !union(label, fullMatrix)) return false
        if (painted.isEmpty) return false
        val target = if (radius == 0f) painted else {
            val halo = Path()
            Paint().apply {
                this.style = Paint.Style.STROKE; strokeWidth = 2 * radius
                strokeJoin = Paint.Join.ROUND; strokeCap = Paint.Cap.ROUND
            }.getFillPath(painted, halo)
            // Path operations can produce even-odd compound contours. Appending
            // fill and stroke contours can cancel coverage; an explicit union
            // keeps every painted component and adds only the requested slop.
            Path().also { if (!it.op(painted, halo, Path.Op.UNION)) return false }
        }
        return Region().apply {
            setPath(target, Region(0, 0, scene.camera.viewportWidth.toInt(), scene.camera.viewportHeight.toInt()))
        }.contains(screen.x.toInt(), screen.y.toInt())
    }

    fun draw(canvas: Canvas, scene: CanvasScene, obj: DrawObject) {
        val style = obj.item.style
        val objectMatrix = EditorDrawing.compose(scene.transformed[obj.item.objectId] ?: obj.item.transform,
            Transform(e = obj.origin.x, f = obj.origin.y))
        val fullMatrix = EditorDrawing.compose(scene.matrix, objectMatrix)
        val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { strokeJoin = Paint.Join.ROUND; strokeCap = Paint.Cap.ROUND }
        fun fill(path: Path, rgba: UInt) {
            val saved = canvas.save()
            try {
                canvas.concat(EditorDrawing.matrix(fullMatrix))
                paint.color = EditorDrawing.argb(rgba); paint.style = Paint.Style.FILL
                canvas.drawPath(path, paint)
            } finally { canvas.restoreToCount(saved) }
        }
        fun outlined(path: Path) {
            style.fill?.let { fill(path, it) }
            // Android's zero strokeWidth is a hairline. Canonical width zero
            // means no outline, including a screen-constant marker.
            if (style.width <= 0.0) return
            val saved = canvas.save()
            try {
                paint.color = EditorDrawing.argb(style.rgba); paint.style = Paint.Style.STROKE
                if (style.screenConstantWidth) {
                    val inDocument = Path(path).apply { transform(EditorDrawing.matrix(objectMatrix)) }
                    canvas.concat(EditorDrawing.matrix(scene.matrix))
                    paint.strokeWidth = (style.width / scene.camera.scale).toFloat()
                    canvas.drawPath(inDocument, paint)
                } else {
                    canvas.concat(EditorDrawing.matrix(fullMatrix))
                    paint.strokeWidth = style.width.toFloat()
                    canvas.drawPath(path, paint)
                }
            } finally { canvas.restoreToCount(saved) }
        }
        outlined(circle); box?.let(::outlined)
        fill(label, style.rgba)
    }
}
internal suspend fun prepareEditorObjects(core: WorkbenchCore, items: List<RenderItem>): List<DrawObject> {
    require(items.size <= 16_384 && items.count { it.shape is Shape.Marker } <= 512)
    val labels = mutableMapOf<String, TextLayout>()
    for (item in items) {
        currentCoroutineContext().ensureActive()
        val shape = item.shape as? Shape.Marker ?: continue
        require(shape.number in 1u..512u)
        labels[item.objectId] = core.layoutText(shape.number.toString(), "Inter", (max(12.0, item.style.width * 4) * 1.15).toFloat())
    }
    return withContext(Dispatchers.Default) {
        items.map { item ->
            currentCoroutineContext().ensureActive()
            val shape = item.shape as? Shape.Marker
            if (shape == null) EditorDrawing.objectPath(item) else {
                val radius = max(12.0, item.style.width * 4)
                val circle = Path().apply { addCircle(0f, 0f, radius.toFloat(), Path.Direction.CW) }
                val box = shape.rectangle?.takeIf { it.width > 0 && it.height > 0 }?.let { rect -> Path().apply {
                    addRect((rect.x - shape.point.x).toFloat(), (rect.y - shape.point.y).toFloat(),
                        (rect.x + rect.width - shape.point.x).toFloat(), (rect.y + rect.height - shape.point.y).toFloat(), Path.Direction.CW)
                } }
                val layout = checkNotNull(labels[item.objectId])
                val label = EditorDrawing.outlinePath(layout.glyphs.flatMap { it.outline }).apply { offset(-layout.width / 2, (radius * 1.15 * .35).toFloat()) }
                DrawObject(item, shape.point, circle, false, marker = MarkerPaint(circle, box, label))
            }
        }
    }
}
