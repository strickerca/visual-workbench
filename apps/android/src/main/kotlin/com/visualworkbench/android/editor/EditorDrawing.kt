@file:OptIn(ExperimentalUnsignedTypes::class)

package com.visualworkbench.android.editor

import android.graphics.Bitmap
import android.graphics.BlendMode
import android.graphics.Canvas
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.Region
import com.visualworkbench.shared.*
import kotlin.math.hypot
import kotlin.math.abs
import kotlin.math.max

/** Immutable after publication to the render thread. Paths use a local f64 origin. */
internal data class DrawObject(val item: RenderItem, val origin: Point, val path: Path,
                               val solid: Boolean, val head: Path? = null)
internal data class CanvasScene(
    val revision: Long,
    val document: DocumentSnapshot,
    val background: Bitmap?,
    val camera: Camera,
    val matrix: Transform,
    val objects: List<DrawObject>,
    val provisional: List<DrawObject> = emptyList(),
    val hidden: Set<String> = emptySet(),
    val transformed: Map<String, Transform> = emptyMap(),
    val selected: String? = null,
    val hover: Point? = null,
    val hoverWidth: Double = 0.0,
    val density: Float = 1f,
    val dark: Boolean = true,
    val peerViewport: List<Point> = emptyList(),
)

internal object EditorDrawing {
    fun argb(rgba: UInt): Int = ((rgba shr 8) or (rgba shl 24)).toInt()
    fun rgba(argb: Long): UInt = ((argb.toUInt() shl 8) or (argb.toUInt() shr 24))

    fun compose(left: Transform, right: Transform) = Transform(
        left.a * right.a + left.c * right.b, left.b * right.a + left.d * right.b,
        left.a * right.c + left.c * right.d, left.b * right.c + left.d * right.d,
        left.a * right.e + left.c * right.f + left.e, left.b * right.e + left.d * right.f + left.f)

    fun map(t: Transform, p: Point) = Point(t.a * p.x + t.c * p.y + t.e, t.b * p.x + t.d * p.y + t.f)
    fun matrix(t: Transform): Matrix = Matrix().apply {
        setValues(floatArrayOf(t.a.toFloat(), t.c.toFloat(), t.e.toFloat(), t.b.toFloat(), t.d.toFloat(), t.f.toFloat(), 0f, 0f, 1f))
    }

    fun contourPath(contours: Contours, origin: Point = Point(0.0, 0.0)): Path {
        require(contours.x.size == contours.y.size && contours.x.size <= 1_000_000)
        val path = Path().apply { fillType = Path.FillType.WINDING }
        var first = 0
        for (endValue in contours.ends) {
            require(endValue <= contours.x.size.toUInt())
            val end = endValue.toInt()
            require(end >= first + 3)
            for (i in first until end) {
                val x = (contours.x[i].toDouble() / 256.0 - origin.x).toFloat()
                val y = (contours.y[i].toDouble() / 256.0 - origin.y).toFloat()
                if (i == first) path.moveTo(x, y) else path.lineTo(x, y)
            }
            path.close(); first = end
        }
        require(first == contours.x.size)
        return path
    }

    fun outlinePath(commands: List<Outline>): Path = Path().apply {
        fillType = Path.FillType.WINDING
        commands.forEach { command -> when (command) {
            is Outline.Move -> moveTo(command.x, command.y)
            is Outline.Line -> lineTo(command.x, command.y)
            is Outline.Quad -> quadTo(command.x1, command.y1, command.x, command.y)
            is Outline.Cubic -> cubicTo(command.x1, command.y1, command.x2, command.y2, command.x, command.y)
            Outline.Close -> close()
        } }
    }

    fun objectPath(item: RenderItem): DrawObject {
        val shape = item.shape
        val origin = when (shape) {
            is Shape.Text -> shape.anchor
            is Shape.Stroke -> Point(item.contours.x.firstOrNull()?.toDouble()?.div(256) ?: 0.0,
                item.contours.y.firstOrNull()?.toDouble()?.div(256) ?: 0.0)
            is Shape.Line -> shape.points.firstOrNull() ?: Point(0.0, 0.0)
            is Shape.Arrow -> shape.points.firstOrNull() ?: Point(0.0, 0.0)
            is Shape.Polygon -> shape.points.firstOrNull() ?: Point(0.0, 0.0)
            is Shape.Rectangle -> Point(shape.rectangle.x, shape.rectangle.y)
            is Shape.Ellipse -> Point(shape.rectangle.x, shape.rectangle.y)
            is Shape.Guide -> Point(shape.rectangle.x, shape.rectangle.y)
            else -> throw UnsupportedOperationException("This document contains an object the Android editor cannot display yet. Its original and edits are preserved.")
        }
        fun poly(points: List<Point>, closed: Boolean = false) = Path().apply {
            points.forEachIndexed { index, p ->
                if (index == 0) moveTo((p.x - origin.x).toFloat(), (p.y - origin.y).toFloat())
                else lineTo((p.x - origin.x).toFloat(), (p.y - origin.y).toFloat())
            }
            if (closed) close()
        }
        fun rect(r: Rect) = RectF(0f, 0f, r.width.toFloat(), r.height.toFloat())
        val path = when (shape) {
            is Shape.Stroke -> contourPath(item.contours, origin)
            is Shape.Text -> outlinePath(shape.outline)
            is Shape.Line -> poly(shape.points)
            is Shape.Arrow -> poly(shape.points)
            is Shape.Polygon -> poly(shape.points, shape.closed)
            is Shape.Rectangle -> Path().apply { addRect(rect(shape.rectangle), Path.Direction.CW) }
            is Shape.Ellipse -> Path().apply { addOval(rect(shape.rectangle), Path.Direction.CW) }
            is Shape.Guide -> Path().apply { addRect(rect(shape.rectangle), Path.Direction.CW) }
            else -> error("Unsupported shape")
        }
        val head = if (shape is Shape.Arrow) {
            val end = shape.points.lastOrNull()
            val before = shape.points.dropLast(1).lastOrNull { it != end }
            if (end == null || before == null) null else {
                val length = hypot(end.x - before.x, end.y - before.y)
                val ux = (end.x - before.x) / length; val uy = (end.y - before.y) / length
                val size = max(item.style.width * 4, 8.0)
                poly(listOf(end, Point(end.x - ux * size - uy * size * .45, end.y - uy * size + ux * size * .45),
                    Point(end.x - ux * size + uy * size * .45, end.y - uy * size - ux * size * .45)), true)
            }
        } else null
        return DrawObject(item, origin, path, shape is Shape.Stroke || shape is Shape.Text, head)
    }

    private fun localMatrix(scene: CanvasScene, obj: DrawObject): Transform = compose(scene.matrix,
        compose(scene.transformed[obj.item.objectId] ?: obj.item.transform, Transform(e = obj.origin.x, f = obj.origin.y)))

    fun screenBounds(scene: CanvasScene, item: RenderItem): List<Point> {
        // Native bounds already include the persisted object transform. Apply
        // only the relative preview transform so handles follow the edited shape.
        val r = item.bounds
        val preview = scene.transformed[item.objectId]
        val original = item.transform
        val determinant = original.a * original.d - original.b * original.c
        val delta = if (preview == null || abs(determinant) < 1e-18) Transform() else {
            val inverse = Transform(original.d / determinant, -original.b / determinant, -original.c / determinant, original.a / determinant,
                (original.c * original.f - original.d * original.e) / determinant, (original.b * original.e - original.a * original.f) / determinant)
            compose(preview, inverse)
        }
        val view = compose(scene.matrix, delta)
        return listOf(Point(r.x, r.y), Point(r.x + r.width, r.y), Point(r.x + r.width, r.y + r.height), Point(r.x, r.y + r.height)).map { map(view, it) }
    }

    private fun drawObject(canvas: Canvas, scene: CanvasScene, obj: DrawObject) {
        val item = obj.item
        val save = canvas.save()
        canvas.concat(matrix(localMatrix(scene, obj)))
        val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = argb(item.style.rgba); strokeJoin = Paint.Join.ROUND; strokeCap = Paint.Cap.ROUND
            strokeWidth = (if (item.style.screenConstantWidth) item.style.width / scene.camera.scale else item.style.width).toFloat()
        }
        val fill = item.style.fill
        if (!obj.solid && fill != null) {
            paint.color = argb(fill); paint.style = Paint.Style.FILL
            canvas.drawPath(obj.path, paint); paint.color = argb(item.style.rgba)
        }
        paint.style = if (obj.solid) Paint.Style.FILL else Paint.Style.STROKE
        canvas.drawPath(obj.path, paint)
        obj.head?.let { paint.style = Paint.Style.FILL; canvas.drawPath(it, paint) }
        canvas.restoreToCount(save)
    }

    /** Full opaque front frame makes multiply wet ink correct across SurfaceControls. */
    fun draw(canvas: Canvas, scene: CanvasScene) {
        canvas.drawColor(if (scene.dark) 0xff17252d.toInt() else 0xffdce5e3.toInt())
        val pageSave = canvas.save()
        canvas.concat(matrix(scene.matrix))
        canvas.clipRect(0f, 0f, scene.document.width.toFloat(), scene.document.height.toFloat())
        canvas.drawColor(android.graphics.Color.WHITE)
        scene.background?.let { canvas.drawBitmap(it, 0f, 0f, Paint(Paint.FILTER_BITMAP_FLAG)) }
        canvas.restoreToCount(pageSave)
        val clipSave = canvas.save()
        val page = Path().apply { addRect(0f, 0f, scene.document.width.toFloat(), scene.document.height.toFloat(), Path.Direction.CW) }
        page.transform(matrix(scene.matrix)); canvas.clipPath(page)
        val combined = (scene.objects + scene.provisional).filter { it.item.objectId !in scene.hidden }
        val orderedLayers = (scene.document.layers.map { it.id } + combined.map { it.item.layerId }).distinct()
        val grouped = combined.groupBy { it.item.layerId }
        for (layerId in orderedLayers) {
            val objects = grouped[layerId] ?: continue
            val first = objects.firstOrNull() ?: continue
            val layer = Paint().apply {
                alpha = (first.item.layerOpacity.coerceIn(0.0, 1.0) * 255).toInt()
                blendMode = if (first.item.layerBlend == "multiply") BlendMode.MULTIPLY else BlendMode.SRC_OVER
            }
            val save = canvas.saveLayer(null, layer)
            objects.forEach { drawObject(canvas, scene, it) }
            canvas.restoreToCount(save)
        }
        canvas.restoreToCount(clipSave)
        if (scene.peerViewport.size == 4) {
            val points = scene.peerViewport.map { map(scene.matrix, it) }
            val outline = Path().apply {
                points.forEachIndexed { index, point -> if (index == 0) moveTo(point.x.toFloat(), point.y.toFloat()) else lineTo(point.x.toFloat(), point.y.toFloat()) }
                close()
            }
            canvas.drawPath(outline, Paint(Paint.ANTI_ALIAS_FLAG).apply {
                color = 0xffc26b18.toInt(); style = Paint.Style.STROKE; strokeWidth = 2 * scene.density
                pathEffect = android.graphics.DashPathEffect(floatArrayOf(8 * scene.density, 5 * scene.density), 0f)
            })
        }
        val selection = scene.objects.firstOrNull { it.item.objectId == scene.selected }
        if (selection != null) {
            val points = screenBounds(scene, selection.item)
            val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = 0xff087dba.toInt(); style = Paint.Style.STROKE; strokeWidth = 2 * scene.density }
            val outline = Path().apply { points.forEachIndexed { index, p -> if (index == 0) moveTo(p.x.toFloat(), p.y.toFloat()) else lineTo(p.x.toFloat(), p.y.toFloat()) }; close() }
            canvas.drawPath(outline, paint)
            paint.style = Paint.Style.FILL
            points.forEach { canvas.drawCircle(it.x.toFloat(), it.y.toFloat(), 6 * scene.density, paint) }
        }
        scene.hover?.let { p ->
            val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = 0xff0883b8.toInt(); style = Paint.Style.STROKE; strokeWidth = scene.density }
            canvas.drawCircle(p.x.toFloat(), p.y.toFloat(), max(2.0 * scene.density, scene.hoverWidth * scene.camera.scale / 2).toFloat(), paint)
        }
    }

    /** Geometry hit testing, never the native conservative AABB alone. */
    fun hit(scene: CanvasScene, screen: Point, radius: Float): DrawObject? = scene.objects.asReversed().firstOrNull { obj ->
        if (obj.item.locked || obj.item.objectId in scene.hidden) false else {
            val path = Path()
            if (obj.solid) path.set(obj.path) else Paint().apply {
                style = if (obj.item.style.fill != null) Paint.Style.FILL_AND_STROKE else Paint.Style.STROKE
                strokeWidth = (if (obj.item.style.screenConstantWidth) obj.item.style.width / scene.camera.scale else obj.item.style.width).toFloat()
                strokeJoin = Paint.Join.ROUND; strokeCap = Paint.Cap.ROUND
            }.getFillPath(obj.path, path)
            obj.head?.let { path.addPath(it) }
            path.transform(matrix(localMatrix(scene, obj)))
            val target = Path()
            val paint = Paint().apply {
                style = Paint.Style.FILL_AND_STROKE
                strokeJoin = Paint.Join.ROUND; strokeCap = Paint.Cap.ROUND
                strokeWidth = radius * 2
            }
            paint.getFillPath(path, target)
            val region = Region()
            val clip = Region(0, 0, scene.camera.viewportWidth.toInt(), scene.camera.viewportHeight.toInt())
            region.setPath(target, clip)
            region.contains(screen.x.toInt(), screen.y.toInt())
        }
    }

    fun eraseHits(scene: CanvasScene, from: Point, to: Point, radius: Float): Set<String> {
        val line = Path().apply { moveTo(from.x.toFloat(), from.y.toFloat()); lineTo(to.x.toFloat(), to.y.toFloat()) }
        val sweep = Path()
        Paint().apply { style = Paint.Style.STROKE; strokeWidth = max(1f, radius * 2); strokeCap = Paint.Cap.ROUND }.getFillPath(line, sweep)
        // A stationary contact still has the same circular eraser footprint.
        sweep.addCircle(to.x.toFloat(), to.y.toFloat(), max(.5f, radius), Path.Direction.CW)
        return scene.objects.mapNotNull { obj ->
            if (obj.item.locked || obj.item.objectId in scene.hidden) return@mapNotNull null
            val local = Path()
            if (obj.solid) local.set(obj.path) else Paint().apply {
                style = if (obj.solid || obj.item.style.fill != null) Paint.Style.FILL_AND_STROKE else Paint.Style.STROKE
                strokeWidth = (if (obj.item.style.screenConstantWidth) obj.item.style.width / scene.camera.scale else obj.item.style.width).toFloat()
                strokeCap = Paint.Cap.ROUND; strokeJoin = Paint.Join.ROUND
            }.getFillPath(obj.path, local)
            obj.head?.let { local.addPath(it) }
            local.transform(matrix(localMatrix(scene, obj)))
            val intersection = Path()
            if (intersection.op(local, sweep, Path.Op.INTERSECT) && !intersection.isEmpty) obj.item.objectId else null
        }.toSet()
    }
}
