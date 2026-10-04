@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import com.visualworkbench.shared.Shape
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import java.awt.BasicStroke
import java.awt.geom.AffineTransform
import java.awt.geom.Area
import java.awt.geom.Ellipse2D
import java.awt.geom.Path2D
import java.awt.geom.Rectangle2D
import kotlin.math.hypot
import kotlin.math.max

/** Whole-object hit test against painted paths, not bounding boxes. Uses Java's
 * existing desktop geometry APIs, with NONZERO winding and canonical fill /
 * round-outline order. No native library, file, clipboard or window is opened. */
internal suspend fun desktopEraseHits(query: EraserHitQuery, core: WorkbenchCore): List<String> = withContext(Dispatchers.Default) {
    admitEraserHit(query)
    val context = currentCoroutineContext()
    val sweep = Path2D.Double().apply {
        moveTo(query.centers.first().x, query.centers.first().y)
        query.centers.drop(1).forEach { lineTo(it.x, it.y) }
    }
    val touched = Area(BasicStroke((query.radius * 2).toFloat(), BasicStroke.CAP_ROUND, BasicStroke.JOIN_ROUND).createStrokedShape(sweep))
    val last = query.centers.last()
    touched.add(Area(Ellipse2D.Double(last.x - query.radius, last.y - query.radius, query.radius * 2, query.radius * 2)))
    touched.intersect(Area(Rectangle2D.Double(0.0, 0.0, query.document.width.toDouble(), query.document.height.toDouble())))
    val ids = mutableListOf<String>()
    for (item in query.document.render.items) {
        context.ensureActive()
        if (item.locked || item.layerOpacity <= 0.0) continue
        val marker = item.shape as? Shape.Marker
        val layout = marker?.let { core.layoutText(it.number.toString(), "Inter", (max(12.0, item.style.width * 4) * 1.15).toFloat()) }
        val painted = desktopPaintedArea(item, query.camera.scale, layout) { context.ensureActive() }
        painted.intersect(touched)
        if (!painted.isEmpty) ids += item.objectId
    }
    ids
}

private fun Transform.awt(): AffineTransform = AffineTransform(a, b, c, d, e, f)
private fun poly(points: List<Point>, closed: Boolean = false, check: () -> Unit): Path2D.Double = Path2D.Double(Path2D.WIND_NON_ZERO).apply {
    points.firstOrNull()?.let { moveTo(it.x, it.y) }
    for (i in 1 until points.size) { if (i % 256 == 0) check(); lineTo(points[i].x, points[i].y) }
    if (closed) closePath()
}
private fun outline(commands: List<Outline>, check: () -> Unit): Path2D.Double = Path2D.Double(Path2D.WIND_NON_ZERO).apply {
    for ((i, command) in commands.withIndex()) {
        if (i % 256 == 0) check()
        when (command) {
            is Outline.Move -> moveTo(command.x.toDouble(), command.y.toDouble())
            is Outline.Line -> lineTo(command.x.toDouble(), command.y.toDouble())
            is Outline.Quad -> quadTo(command.x1.toDouble(), command.y1.toDouble(), command.x.toDouble(), command.y.toDouble())
            is Outline.Cubic -> curveTo(command.x1.toDouble(), command.y1.toDouble(), command.x2.toDouble(), command.y2.toDouble(), command.x.toDouble(), command.y.toDouble())
            Outline.Close -> closePath()
        }
    }
}

internal fun desktopPaintedArea(item: RenderItem, scale: Double, markerText: TextLayout? = null, check: () -> Unit = {}): Area {
    val result = Area(); val transform = item.transform.awt()
    fun fill(path: java.awt.Shape, rgba: UInt) { if ((rgba and 255u) != 0u) result.add(Area(transform.createTransformedShape(path))) }
    fun stroke(path: java.awt.Shape) {
        if (item.style.width <= 0 || (item.style.rgba and 255u) == 0u) return
        if (item.style.screenConstantWidth) {
            result.add(Area(BasicStroke((item.style.width / scale).toFloat(), BasicStroke.CAP_ROUND, BasicStroke.JOIN_ROUND)
                .createStrokedShape(transform.createTransformedShape(path))))
        } else result.add(Area(transform.createTransformedShape(BasicStroke(item.style.width.toFloat(), BasicStroke.CAP_ROUND, BasicStroke.JOIN_ROUND).createStrokedShape(path))))
    }
    fun outlined(path: java.awt.Shape) { item.style.fill?.let { fill(path, it) }; stroke(path) }
    fun rectangle(value: Rect): Rectangle2D.Double = Rectangle2D.Double(value.x, value.y, value.width, value.height)
    when (val shape = item.shape) {
        is Shape.Stroke -> {
            val c = item.contours; require(c.x.size == c.y.size)
            val path = Path2D.Double(Path2D.WIND_NON_ZERO); var first = 0
            for (raw in c.ends) {
                val end = raw.toLong(); require(end in (first + 3).toLong()..c.x.size.toLong())
                path.moveTo(c.x[first] / 256.0, c.y[first] / 256.0)
                for (i in first + 1 until end.toInt()) { if (i % 256 == 0) check(); path.lineTo(c.x[i] / 256.0, c.y[i] / 256.0) }
                path.closePath(); first = end.toInt()
            }
            require(first == c.x.size); fill(path, item.style.rgba)
        }
        is Shape.Text -> fill(outline(shape.outline, check).apply { transform(AffineTransform.getTranslateInstance(shape.anchor.x, shape.anchor.y)) }, item.style.rgba)
        is Shape.Line -> stroke(poly(shape.points, check = check))
        is Shape.Arrow -> {
            stroke(poly(shape.points, check = check))
            val end = shape.points.lastOrNull(); val before = shape.points.dropLast(1).lastOrNull { it != end }
            if (end != null && before != null) {
                val length = hypot(end.x - before.x, end.y - before.y); val ux = (end.x - before.x) / length; val uy = (end.y - before.y) / length
                val size = max(8.0, item.style.width * 4)
                fill(poly(listOf(end, Point(end.x - ux * size - uy * size * .45, end.y - uy * size + ux * size * .45),
                    Point(end.x - ux * size + uy * size * .45, end.y - uy * size - ux * size * .45)), true, check), item.style.rgba)
            }
        }
        is Shape.Polygon -> outlined(poly(shape.points, true, check))
        is Shape.Rectangle -> outlined(rectangle(shape.rectangle))
        is Shape.Ellipse -> outlined(Ellipse2D.Double(shape.rectangle.x, shape.rectangle.y, shape.rectangle.width, shape.rectangle.height))
        is Shape.Guide -> stroke(rectangle(shape.rectangle))
        is Shape.Marker -> {
            val radius = max(12.0, item.style.width * 4)
            outlined(Ellipse2D.Double(shape.point.x - radius, shape.point.y - radius, radius * 2, radius * 2))
            shape.rectangle?.let { outlined(rectangle(it)) }
            markerText?.let { layout -> fill(outline(layout.glyphs.flatMap { it.outline }, check).apply {
                transform(AffineTransform.getTranslateInstance(shape.point.x - layout.width / 2, shape.point.y + radius * 1.15 * .35))
            }, item.style.rgba) }
        }
        is Shape.Result, Shape.Adjustment -> throw WorkflowFailure(WorkflowFailureKind.Invalid)
    }
    return result
}
