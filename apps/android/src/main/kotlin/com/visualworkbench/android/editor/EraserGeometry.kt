package com.visualworkbench.android.editor

import android.graphics.Paint
import android.graphics.Path
import com.visualworkbench.shared.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

/** Uses the editor's canonical path builder, then applies the same complete
 * object transform and D-space / constant-screen-width stroke policy. No peer
 * or predicted geometry can enter this immutable snapshot query. */
internal suspend fun androidEraseHits(query: EraserHitQuery): List<String> = withContext(Dispatchers.Default) {
    admitEraserHit(query)
    val context = currentCoroutineContext()
    val line = Path().apply {
        moveTo(query.centers.first().x.toFloat(), query.centers.first().y.toFloat())
        query.centers.drop(1).forEach { lineTo(it.x.toFloat(), it.y.toFloat()) }
    }
    val sweep = Path()
    Paint().apply { style = Paint.Style.STROKE; strokeWidth = (query.radius * 2).toFloat()
        strokeJoin = Paint.Join.ROUND; strokeCap = Paint.Cap.ROUND }.getFillPath(line, sweep)
    val last = query.centers.last()
    val end = Path().apply { addCircle(last.x.toFloat(), last.y.toFloat(), query.radius.toFloat(), Path.Direction.CW) }
    if (!sweep.op(end, Path.Op.UNION)) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    val page = Path().apply { addRect(0f, 0f, query.document.width.toFloat(), query.document.height.toFloat(), Path.Direction.CW) }
    if (!sweep.op(page, Path.Op.INTERSECT)) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    val ids = mutableListOf<String>()
    for (item in query.document.render.items) {
        context.ensureActive()
        if (item.locked || item.layerOpacity <= 0.0) continue
        val painted = androidPaintedPath(item, query.camera.scale)
        val intersection = Path()
        if (!intersection.op(painted, sweep, Path.Op.INTERSECT)) throw WorkflowFailure(WorkflowFailureKind.Invalid)
        if (!intersection.isEmpty) ids += item.objectId
    }
    ids
}

internal fun androidPaintedPath(item: RenderItem, cameraScale: Double): Path {
    val obj = EditorDrawing.objectPath(item)
    val matrix = EditorDrawing.matrix(EditorDrawing.compose(item.transform, Transform(e = obj.origin.x, f = obj.origin.y)))
    val result = Path().apply { fillType = Path.FillType.WINDING }
    fun union(path: Path) {
        // Fill, outline and arrow head are independent paint calls. Opposite
        // winding must never cancel their overlapping painted pixels. Each
        // component retains its own NONZERO holes before the geometric union.
        if (!result.op(path, Path.Op.UNION)) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    }
    fun fill(path: Path, rgba: UInt) {
        if ((rgba and 255u) != 0u) union(Path(path).apply { transform(matrix) })
    }
    if (obj.solid) fill(obj.path, item.style.rgba)
    else {
        if (item.shape is Shape.Rectangle || item.shape is Shape.Ellipse || item.shape is Shape.Polygon)
            item.style.fill?.let { fill(obj.path, it) }
        if (item.style.width > 0 && (item.style.rgba and 255u) != 0u) {
            val path = Path(obj.path)
            if (item.style.screenConstantWidth) path.transform(matrix)
            val outline = Path()
            val ok = Paint().apply {
                style = Paint.Style.STROKE; strokeCap = Paint.Cap.ROUND; strokeJoin = Paint.Join.ROUND
                strokeWidth = (if (item.style.screenConstantWidth) item.style.width / cameraScale else item.style.width).toFloat()
            }.getFillPath(path, outline)
            if (!ok) throw WorkflowFailure(WorkflowFailureKind.Invalid)
            if (!item.style.screenConstantWidth) outline.transform(matrix)
            union(outline)
        }
    }
    obj.head?.let { fill(it, item.style.rgba) }
    return result
}
