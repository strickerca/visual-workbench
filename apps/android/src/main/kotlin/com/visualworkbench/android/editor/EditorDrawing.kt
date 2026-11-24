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
import com.visualworkbench.android.documents.PreparedDocumentTile
import com.visualworkbench.android.documents.drawDocumentTiles
import kotlin.math.hypot
import kotlin.math.abs
import kotlin.math.max

/** The saved objects of a scene that could put a pixel inside a document-space
 * rectangle grown by a margin, in drawing order; a superset is allowed. Null:
 * every saved object has to be looked at. */
internal typealias SavedNear = (visible: Rect, margin: Double) -> List<DrawObject>?

/** Immutable after publication to the render thread. Paths use a local f64 origin. */
internal data class DrawObject(val item: RenderItem, val origin: Point, val path: Path,
                               val solid: Boolean, val head: Path? = null, val marker: MarkerPaint? = null)
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
    val aiResults: Map<String,AiResultImage<AiBitmap>> = emptyMap(),
    val resultDisplayReady: Boolean = true,
    val precision: PrecisionFrame = PrecisionFrame(),
    val smoothBackground: Boolean = true,
    val documentFrame:DocumentDrawFrame<PreparedDocumentTile>?=null,
    val documentTiles:List<PreparedDocumentTile> = emptyList(),
    val documentDraft:Pair<Point,Point>?=null,
    val giantMode:Boolean=false,
    val giantFrame:GiantVisibleFrame<Bitmap>?=null,
    val rotationHandle:ObjectRotationHandle?=null,
    /** Diagnostics only: input-clock time of the newest real pen sample in the
     * wet ink of this scene and of that stroke's pen-down; 0 without wet ink. */
    val wetSampleMs: Long = 0,
    val wetStartMs: Long = 0,
)

/** The path commands of a stroke's contours, in f32 coordinates around an f64
 * origin: per contour one move, its lines and a close. Every stroke path is
 * built with this arithmetic, from arrays and from packed contours alike. */
internal interface ContourCommands : ContourVisitor {
    val originX: Double
    val originY: Double
    /** Whether the current contour has had its first vertex. */
    var started: Boolean
    fun moveTo(x: Float, y: Float)
    fun lineTo(x: Float, y: Float)
    fun close()
    override fun begin(vertexCount: Int) { require(vertexCount >= 3); started = false }
    override fun vertex(x: Long, y: Long) {
        val px = (x.toDouble() / 256.0 - originX).toFloat()
        val py = (y.toDouble() / 256.0 - originY).toFloat()
        if (started) lineTo(px, py) else { moveTo(px, py); started = true }
    }
    override fun end() = close()
}

/** A stroke path that takes its own contours, so building it allocates
 * nothing besides the path. */
internal class ContourPath(override val originX: Double, override val originY: Double) : Path(), ContourCommands {
    override var started: Boolean = false
    override fun moveTo(x: Float, y: Float) = super.moveTo(x, y)
    override fun lineTo(x: Float, y: Float) = super.lineTo(x, y)
    override fun close() = super.close()
}

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

    /** Reads [contours] in whichever form it is held, without unpacking or
     * boxing it; the path itself is the only allocation. */
    fun contourPath(contours: Contours, origin: Point = Point(0.0, 0.0)): Path {
        require(contours.vertexCount <= 1_000_000)
        val path = ContourPath(origin.x, origin.y).apply { fillType = Path.FillType.WINDING }
        contours.visit(path)
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

    /** The local origin [objectPath] builds [item]'s path around. */
    fun objectOrigin(item: RenderItem): Point = when (val shape = item.shape) {
        is Shape.Result -> Point(0.0,0.0)
        is Shape.Text -> shape.anchor
        is Shape.Stroke -> Point(item.contours.firstX.toDouble() / 256, item.contours.firstY.toDouble() / 256)
        is Shape.Line -> shape.points.firstOrNull() ?: Point(0.0, 0.0)
        is Shape.Arrow -> shape.points.firstOrNull() ?: Point(0.0, 0.0)
        is Shape.Polygon -> shape.points.firstOrNull() ?: Point(0.0, 0.0)
        is Shape.Rectangle -> Point(shape.rectangle.x, shape.rectangle.y)
        is Shape.Ellipse -> Point(shape.rectangle.x, shape.rectangle.y)
        is Shape.Guide -> Point(shape.rectangle.x, shape.rectangle.y)
        else -> throw UnsupportedOperationException("This document contains an object the Android editor cannot display yet. Its original and edits are preserved.")
    }

    fun objectPath(item: RenderItem): DrawObject {
        val shape = item.shape
        val origin = objectOrigin(item)
        fun poly(points: List<Point>, closed: Boolean = false) = Path().apply {
            points.forEachIndexed { index, p ->
                if (index == 0) moveTo((p.x - origin.x).toFloat(), (p.y - origin.y).toFloat())
                else lineTo((p.x - origin.x).toFloat(), (p.y - origin.y).toFloat())
            }
            if (closed) close()
        }
        fun rect(r: Rect) = RectF(0f, 0f, r.width.toFloat(), r.height.toFloat())
        val path = when (shape) {
            is Shape.Result -> Path() // Pixels own this shape; never fabricate a vector outline.
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
        if(item.shape is Shape.Result){
            val pixels=scene.aiResults[item.objectId]?:error("Unadmitted Result frame")
            drawAiResult(canvas,item,pixels,scene.matrix,scene.transformed[item.objectId]?:item.transform);return
        }
        obj.marker?.let { marker -> marker.draw(canvas, scene, obj); return }
        val save = canvas.save()
        val objectMatrix = compose(scene.transformed[item.objectId] ?: item.transform,
            Transform(e = obj.origin.x, f = obj.origin.y))
        val constantOutline = !obj.solid && item.style.screenConstantWidth
        // A constant-screen outline is stroked after the complete object
        // affine. Only the camera may scale its document-space width here.
        val path = if (constantOutline) Path(obj.path).apply { transform(matrix(objectMatrix)) } else obj.path
        canvas.concat(matrix(if (constantOutline) scene.matrix else compose(scene.matrix, objectMatrix)))
        val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = argb(item.style.rgba); strokeJoin = Paint.Join.ROUND; strokeCap = Paint.Cap.ROUND
            strokeWidth = (if (item.style.screenConstantWidth) item.style.width / scene.camera.scale else item.style.width).toFloat()
        }
        val fill = item.style.fill
        val filledShape = item.shape is Shape.Rectangle || item.shape is Shape.Ellipse || item.shape is Shape.Polygon
        if (!obj.solid && filledShape && fill != null) {
            paint.color = argb(fill); paint.style = Paint.Style.FILL
            canvas.drawPath(path, paint); paint.color = argb(item.style.rgba)
        }
        // Canonical erased strokes are fill-only compound polygons. Width
        // zero must skip the outline, not request Android's one-pixel hairline.
        if (obj.solid || item.style.width > 0.0) {
            paint.style = if (obj.solid) Paint.Style.FILL else Paint.Style.STROKE
            canvas.drawPath(path, paint)
        }
        obj.head?.let {
            paint.style = Paint.Style.FILL
            val head = if (constantOutline) Path(it).apply { transform(matrix(objectMatrix)) } else it
            canvas.drawPath(head, paint)
        }
        canvas.restoreToCount(save)
    }

    /** Canvas surround outside the page; also the canvas host's backdrop while no frame is committed. */
    fun surroundArgb(dark: Boolean): Int = if (dark) 0xff17252d.toInt() else 0xffdce5e3.toInt()
    /** Full opaque front frame makes multiply wet ink correct across SurfaceControls. */
    fun draw(canvas: Canvas, scene: CanvasScene) {
        if (drawDocument(canvas, scene, scene.provisional, null)) drawOverlays(canvas, scene)
    }

    /** As [draw], leaving out saved objects that cannot touch [visible] (a
     * document-space rectangle). Used for the magnifier panes of an ink frame,
     * which show a small part of the document. */
    fun drawVisible(canvas: Canvas, scene: CanvasScene, visible: Rect, near: SavedNear? = null) {
        if (drawDocument(canvas, scene, scene.provisional, visible, near)) drawOverlays(canvas, scene)
    }

    /** The surround and the white page under the document. The page is
     * presentation only: the source and every object layer go into one
     * transparent stack over it, so that a Result replacement keeps the
     * source's alpha. */
    private fun drawPage(canvas: Canvas, scene: CanvasScene) {
        canvas.drawColor(surroundArgb(scene.dark))
        val pageSave = canvas.save()
        canvas.concat(matrix(scene.matrix))
        canvas.clipRect(0f, 0f, scene.document.width.toFloat(), scene.document.height.toFloat())
        canvas.drawColor(android.graphics.Color.WHITE)
        canvas.restoreToCount(pageSave)
    }

    /** Clips to the page as it lies on the screen. */
    private fun clipToPage(canvas: Canvas, scene: CanvasScene) {
        val page = Path().apply { addRect(0f, 0f, scene.document.width.toFloat(), scene.document.height.toFloat(), Path.Direction.CW) }
        page.transform(matrix(scene.matrix)); canvas.clipPath(page)
    }

    /** Surround, page, source pixels and every object layer. False: results are
     * not ready and nothing more may be drawn over the page. With [visible],
     * saved objects whose bounds lie wholly outside it are skipped; they would
     * be clipped away entirely, so the pixels are the same. */
    private fun drawDocument(canvas: Canvas, scene: CanvasScene, provisional: List<DrawObject>, visible: Rect?, near: SavedNear? = null): Boolean {
        drawPage(canvas, scene)
        val clipSave = canvas.save()
        clipToPage(canvas, scene)
        if(!scene.resultDisplayReady){canvas.restoreToCount(clipSave);return false}
        val documentLayer=canvas.saveLayer(null,null)
        drawStack(canvas, scene, provisional, visible, near)
        canvas.restoreToCount(documentLayer)
        canvas.restoreToCount(clipSave)
        return true
    }

    // The kept picture. It holds exactly what [drawDocument] puts into its
    // document layer for the saved objects, so a frame drawn from it goes
    // through the same steps as a whole scene: surround, page, the stack laid
    // on through the page's edge, overlays.

    /** A kept picture from nothing: the saved document stack on a transparent
     * ground, for a scene [pictureRefusal] lets through. */
    fun drawStack(canvas: Canvas, scene: CanvasScene) = drawStack(canvas, scene, emptyList(), null)

    /** A kept picture from the one [before], which lacks only [added]: the
     * same pixels, and those objects drawn over them. Only valid when they
     * are drawn after everything else in a plainly composited layer
     * ([DryLayers.inkOnTop]). What is drawn is the added objects alone,
     * whatever the document holds. */
    fun drawAdded(canvas: Canvas, scene: CanvasScene, before: Bitmap, added: List<DrawObject>) {
        canvas.drawBitmap(before, 0f, 0f, PICTURE_COPY)
        // In a layer of their own, as a whole scene draws its object layers:
        // the renderer then draws their outlines the same way it does there.
        val layer = canvas.saveLayer(null, null)
        added.forEach { if (it.item.objectId !in scene.hidden) drawObject(canvas, scene, it) }
        canvas.restoreToCount(layer)
    }

    /** One frame from the kept picture [kept] of [scene]'s saved objects:
     * the page, the picture laid on through the page's edge, the ink in
     * progress over it and the overlays. The ink goes straight over the
     * picture, which is the same as drawing it into the stack only when it is
     * in the layer drawn last and that layer is composited plainly
     * ([DryLayers.inkOnTop]). */
    fun drawKept(canvas: Canvas, scene: CanvasScene, kept: Bitmap, selection: DrawObject? = selectedObject(scene)) {
        drawPage(canvas, scene)
        val clipSave = canvas.save()
        clipToPage(canvas, scene)
        canvas.drawBitmap(kept, 0f, 0f, PICTURE_OVER)
        scene.provisional.forEach { if (it.item.objectId !in scene.hidden) drawObject(canvas, scene, it) }
        canvas.restoreToCount(clipSave)
        drawOverlays(canvas, scene, selection)
    }

    // A part of the screen drawn again. Composition is per pixel: inside a
    // clip of whole pixels, the stack drawn from the saved objects that can
    // reach that part is the stack drawn from all of them, pixel for pixel,
    // as long as each layer is composited the same way as in the whole scene.
    // It is when every object of a layer shares that layer's compositing and
    // the document lists every layer drawn ([DryLayers.clean]): the layer is
    // then composited the same whichever of its objects comes first, and the
    // layers come in the document's order. A layer with no object there is
    // left out, as an empty layer changes no pixel.

    /** A kept picture from the one [before], with the pixel boxes [boxes]
     * cleared and drawn again: each from [parts] at the same position, the
     * saved objects that can reach it in drawing order ([touching]). The same
     * pixels as [drawStack] of the whole scene, everywhere. */
    fun drawPatched(canvas: Canvas, scene: CanvasScene, before: Bitmap, boxes: List<PixelBox>, parts: List<List<DrawObject>>) {
        canvas.drawBitmap(before, 0f, 0f, PICTURE_COPY)
        for (at in boxes.indices) {
            val box = boxes[at]
            val save = canvas.save()
            canvas.clipRect(box.left, box.top, box.right, box.bottom)
            canvas.drawColor(android.graphics.Color.TRANSPARENT, BlendMode.CLEAR)
            drawStack(canvas, scene, parts[at], emptyList())
            canvas.restoreToCount(save)
        }
    }

    /** One frame from the kept picture [kept], with the pixel boxes [boxes]
     * drawn again over it from [parts] (as [drawPatched]): the picture where
     * nothing differs from it, and what a gesture changes (a stroke being
     * moved or turned, objects hidden while erased) where something does.
     * Then the ink in progress and the overlays, as [drawKept]. */
    fun drawOver(canvas: Canvas, scene: CanvasScene, kept: Bitmap, boxes: List<PixelBox>, parts: List<List<DrawObject>>,
                 selection: DrawObject? = selectedObject(scene)) {
        drawPage(canvas, scene)
        val clipSave = canvas.save()
        clipToPage(canvas, scene)
        val pictureSave = canvas.save()
        for (box in boxes) canvas.clipOutRect(box.left, box.top, box.right, box.bottom)
        canvas.drawBitmap(kept, 0f, 0f, PICTURE_OVER)
        canvas.restoreToCount(pictureSave)
        for (at in boxes.indices) {
            val box = boxes[at]
            val save = canvas.save()
            canvas.clipRect(box.left, box.top, box.right, box.bottom)
            // The document layer of a whole scene, here only as large as the box.
            canvas.saveLayer(null, null)
            drawStack(canvas, scene, parts[at], emptyList())
            canvas.restoreToCount(save)
        }
        scene.provisional.forEach { if (it.item.objectId !in scene.hidden) drawObject(canvas, scene, it) }
        canvas.restoreToCount(clipSave)
        drawOverlays(canvas, scene, selection)
    }

    /** Of [candidates] (saved objects of [scene] in drawing order, among them
     * every one that could reach [visible]), those that could put a pixel
     * inside the document-space rectangle [visible]. */
    fun touching(scene: CanvasScene, candidates: List<DrawObject>, visible: Rect): List<DrawObject> = candidates.filter { mayTouch(scene, it, visible) }

    /** How far beyond its native box a saved object may be taken to reach, in
     * document units: four screen pixels at [scene]'s scale. */
    fun cullMargin(scene: CanvasScene): Double = 4.0 / scene.camera.scale

    /** A picture is drawn pixel for pixel, never smoothed. Made at first use:
     * the unit tests use this object without a graphics framework. */
    private val PICTURE_OVER by lazy { Paint().apply { isFilterBitmap = false; isAntiAlias = false; isDither = false } }
    private val PICTURE_COPY by lazy { Paint(PICTURE_OVER).apply { blendMode = BlendMode.SRC } }

    /** The document as it is stacked, on whatever is beneath: the source
     * pixels, then every object layer in order, each composited as a whole. */
    private fun drawStack(canvas: Canvas, scene: CanvasScene, provisional: List<DrawObject>, visible: Rect?, near: SavedNear? = null) {
        // With [near], only the saved objects it names are looked at: it names
        // every one that could touch [visible], in drawing order, and the same
        // test as without it then picks the same ones.
        val saved = if (visible == null) scene.objects
            else (near?.invoke(visible, cullMargin(scene)) ?: scene.objects).filter { mayTouch(scene, it, visible) }
        drawStack(canvas, scene, saved, provisional)
    }

    /** [drawStack] of the saved objects [saved] (in drawing order) and [provisional]. */
    private fun drawStack(canvas: Canvas, scene: CanvasScene, saved: List<DrawObject>, provisional: List<DrawObject>) {
        val sourceSave=canvas.save()
        canvas.concat(matrix(scene.matrix))
        scene.giantFrame?.let{drawGiantAndroidTiles(canvas,it)}
        scene.background?.let{canvas.drawBitmap(it,0f,0f,Paint(if(scene.smoothBackground) Paint.FILTER_BITMAP_FLAG else 0))}
        scene.documentFrame?.let{drawDocumentTiles(canvas,scene.documentTiles,it.binding)}
        scene.documentDraft?.let{(a,b)->
            val paint=Paint(Paint.ANTI_ALIAS_FLAG).apply{color=0xffecc769.toInt();style=Paint.Style.STROKE;strokeWidth=(1.0/scene.camera.scale).toFloat()}
            canvas.drawRect(minOf(a.x,b.x).toFloat(),minOf(a.y,b.y).toFloat(),maxOf(a.x,b.x).toFloat(),maxOf(a.y,b.y).toFloat(),paint)
        }
        canvas.restoreToCount(sourceSave)
        val combined = (saved + provisional).filter { it.item.objectId !in scene.hidden }
        val orderedLayers = (scene.document.layers.map { it.id } + combined.map { it.item.layerId }).distinct()
        val grouped = combined.groupBy { it.item.layerId }
        for (layerId in orderedLayers) {
            val objects = grouped[layerId] ?: continue
            val first = objects.firstOrNull() ?: continue
            if(first.item.shape is Shape.Result){
                require(objects.size==1)
                drawObject(canvas,scene,first);continue
            }
            val layer = Paint().apply {
                alpha = (first.item.layerOpacity.coerceIn(0.0, 1.0) * 255).toInt()
                blendMode = if (first.item.layerBlend == "multiply") BlendMode.MULTIPLY else BlendMode.SRC_OVER
            }
            val save = canvas.saveLayer(null, layer)
            objects.forEach { drawObject(canvas, scene, it) }
            canvas.restoreToCount(save)
        }
    }

    /** Whether a saved object could put a pixel inside [visible]. Only
     * objects whose native bounds hold everything drawn for them are ever
     * skipped ([cullable]). Markers, results, outlines of constant screen
     * width and anything under a preview transform are always drawn. */
    private fun mayTouch(scene: CanvasScene, obj: DrawObject, visible: Rect): Boolean {
        val item = obj.item
        if (!cullable(item, obj.marker != null) || item.objectId in scene.transformed) return true
        return boundsMayTouch(item.bounds, visible, cullMargin(scene))
    }

    /** Whether [item] may ever be left out of a part of the page by its
     * native box alone ([mayTouch]): a stroke is a filled outline, and its box
     * holds that outline whole; a shape's box (line, arrow, polygon,
     * rectangle, ellipse, guide) is grown by twice its width, more than the
     * half width its outline reaches (and an arrow's by more than its head);
     * text is a filled outline, and its box holds every point of the glyphs'
     * curves. An outline whose width is constant on the screen reaches farther
     * in the document the further out the page is zoomed, so it is not. */
    fun cullable(item: RenderItem, marker: Boolean): Boolean = !marker && !item.style.screenConstantWidth && boxHoldsDrawing(item)

    /** The saved object [scene] has selected, found by a pass over its saved
     * objects; a frame from the kept picture finds it by its id instead. */
    fun selectedObject(scene: CanvasScene): DrawObject? = scene.selected?.let { id -> scene.objects.firstOrNull { it.item.objectId == id } }

    /** Peer viewport outline, rotation handle, selection and hover: drawn
     * after the document. [selection] is the saved object [scene] has selected. */
    fun drawOverlays(canvas: Canvas, scene: CanvasScene, selection: DrawObject? = selectedObject(scene)) {
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
        scene.rotationHandle?.let{handle->
            val pivot=map(scene.matrix,handle.pivot);val point=map(scene.matrix,handle.point)
            val paint=Paint(Paint.ANTI_ALIAS_FLAG).apply{color=0xff81e0c8.toInt();style=Paint.Style.STROKE;strokeWidth=1.5f*scene.density}
            canvas.drawLine(pivot.x.toFloat(),pivot.y.toFloat(),point.x.toFloat(),point.y.toFloat(),paint)
            canvas.drawCircle(point.x.toFloat(),point.y.toFloat(),6f*scene.density,paint)
        }
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
        if (obj.item.locked || obj.item.objectId in scene.hidden) false
        else if(obj.item.shape is Shape.Result) aiResultHit(scene,obj.item,screen,radius) else if (obj.marker != null) obj.marker.hit(scene, obj, screen, radius)
        else {
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
