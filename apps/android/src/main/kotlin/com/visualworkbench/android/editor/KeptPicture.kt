package com.visualworkbench.android.editor

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Path
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import com.visualworkbench.android.precision.PrecisionDrawing
import com.visualworkbench.shared.DocumentSnapshot
import com.visualworkbench.shared.Point
import com.visualworkbench.shared.Rect
import com.visualworkbench.shared.RenderItem
import com.visualworkbench.shared.Shape
import com.visualworkbench.shared.Transform
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicLongArray
import kotlin.math.abs
import kotlin.math.round

// The canvas shows kept pictures of the saved page, as a paint program does,
// one for each layer that draws anything, composited for every frame with the
// layer's opacity and blend. A finished stroke is drawn into its layer's
// picture once, whichever layer it is in; a change to the page (undo, redo, an
// erasure, a move, a turn, a deletion, a restyle, a change from the paired
// computer) draws again only the part of the pictures it touched; hiding a
// layer, or changing its opacity, blend or place, only composites the pictures
// otherwise. While the page is panned, zoomed or turned, frames draw the
// pictures moved, scaled or turned, and fill what they do not cover from a
// soft picture of the whole page; once the view rests the pictures are drawn
// at it, and the frame is the whole scene again. The rules are plain
// functions here and in TouchedArea.kt, tested without a phone; the drawing
// is in EditorDrawing.

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

/** Whether a layer whose first drawn object is [a] is composited as one whose
 * first drawn object is [b]: the same opacity as drawn, the same blend, and
 * neither or both an AI result (which is drawn by its own route). */
internal fun samePaint(a: RenderItem, b: RenderItem): Boolean = layerAlpha(a) == layerAlpha(b) &&
    (a.layerBlend == "multiply") == (b.layerBlend == "multiply") && (a.shape is Shape.Result) == (b.shape is Shape.Result)
private fun layerAlpha(item: RenderItem): Int = (item.layerOpacity.coerceIn(0.0, 1.0) * 255).toInt()

/** Whether [a] and [b] draw the same pixels in their layer's own picture:
 * the same object, drawn alike. How their layer is composited (opacity,
 * blend) and whether it is locked do not count. A stroke read again with its
 * layer changed keeps its contours (and so its path); any other kind of
 * object is compared field for field, and has no contours. */
internal fun sameDrawing(a: RenderItem, b: RenderItem): Boolean = a === b || (a.objectId == b.objectId && a.layerId == b.layerId &&
    a.bounds == b.bounds && a.transform == b.transform && a.style == b.style && a.shape == b.shape &&
    (a.contours === b.contours || (a.contours.vertexCount == 0 && a.contours.contourCount == 0 && b.contours.vertexCount == 0 && b.contours.contourCount == 0)))

// Layers.

/** The saved objects of one list by layer, each layer's in list order, and
 * the order the layers are drawn in: the document's layers, then any other in
 * first-use order, as the whole scene draws them; and the box around all of
 * them ([bounds], document space; null when one cannot be judged). Worked out
 * once per list. */
internal class LayerLists<T>(val order: List<String>, private val lists: Map<String, List<T>>, val bounds: Rect? = null,
                             /** When each layer's objects lie together in the list: the runs, in list order. */
                             val runs: List<LayerRun>? = null) {
    fun of(layer: String): List<T> = lists[layer] ?: emptyList()
    val layers: Set<String> get() = lists.keys
}

/** The objects of [layer] lie at [start] until [end] of a list of saved objects. */
internal data class LayerRun(val layer: String, val start: Int, val end: Int)

/** The layers drawn in order, from where their objects lie: the document's
 * layers in its order, then any other in first-use order. */
private fun layerOrder(layerIds: List<String>, runs: List<LayerRun>): List<String> {
    val present = HashSet<String>(); for (run in runs) present.add(run.layer)
    val order = ArrayList<String>(runs.size)
    for (id in layerIds) if (id in present) order.add(id)
    if (order.size < runs.size) { val listed = layerIds.toHashSet(); for (run in runs) if (run.layer !in listed) order.add(run.layer) }
    return order
}

/** Adds the run [layer] at [start] until [end] to [runs], joined with the one before when they meet in one layer. */
private fun pushRun(runs: ArrayList<LayerRun>, layer: String, start: Int, end: Int) {
    if (start >= end) return
    val last = runs.lastOrNull()
    if (last != null && last.end == start && last.layer == layer) runs[runs.size - 1] = LayerRun(layer, last.start, end) else runs.add(LayerRun(layer, start, end))
}

/** [held]'s lists, of [old], brought to [new], which differs from it in a
 * middle part: the runs of the common start and end are taken as they were,
 * and only the middle is looked at. Each layer's list is then a view of its
 * run of [new]. Null when [held] has no runs, when more than [limit] objects
 * changed, or when a layer's objects no longer lie together. */
internal fun <T> followLayerLists(held: LayerLists<T>, old: List<T>, new: List<T>, layerIds: List<String>, limit: Int, item: (T) -> RenderItem): LayerLists<T>? {
    val runs = held.runs ?: return null
    val shorter = minOf(old.size, new.size)
    var start = 0
    while (start < shorter && old[start] === new[start]) start++
    var oldEnd = old.size; var newEnd = new.size
    while (oldEnd > start && newEnd > start && old[oldEnd - 1] === new[newEnd - 1]) { oldEnd--; newEnd-- }
    if (newEnd - start > limit || oldEnd - start > limit) return null
    val shift = newEnd - oldEnd
    val next = ArrayList<LayerRun>(runs.size + 1)
    for (run in runs) if (run.start < start) pushRun(next, run.layer, run.start, minOf(run.end, start))
    var bounds = held.bounds
    for (at in start until newEnd) {
        val shown = item(new[at])
        pushRun(next, shown.layerId, at, at + 1)
        val box = shown.bounds
        val known = bounds
        bounds = if (known == null || !(box.x.isFinite() && box.y.isFinite() && box.width.isFinite() && box.height.isFinite()) || box.width < 0 || box.height < 0) null
            else { val left = minOf(known.x, box.x); val top = minOf(known.y, box.y)
                Rect(left, top, maxOf(known.x + known.width, box.x + box.width) - left, maxOf(known.y + known.height, box.y + box.height) - top) }
    }
    for (run in runs) if (run.end > oldEnd) pushRun(next, run.layer, maxOf(run.start, oldEnd) + shift, run.end + shift)
    val lists = HashMap<String, List<T>>(next.size * 2)
    for (run in next) if (lists.put(run.layer, new.subList(run.start, run.end)) != null) return null
    // The box may only grow: a box too large says less often that a moved picture is exact, never wrongly.
    return LayerLists(layerOrder(layerIds, next), lists, bounds, next)
}

internal fun <T> layerLists(layerIds: List<String>, objects: List<T>, item: (T) -> RenderItem): LayerLists<T> {
    val lists = LinkedHashMap<String, ArrayList<T>>()
    var left = Double.POSITIVE_INFINITY; var top = Double.POSITIVE_INFINITY; var right = Double.NEGATIVE_INFINITY; var bottom = Double.NEGATIVE_INFINITY
    var judged = true
    // Objects of one layer come together: the list of the one before is tried first, without hashing its id.
    var lastId: String? = null; var lastList: ArrayList<T>? = null
    for (index in objects.indices) {
        val one = objects[index]; val shown = item(one)
        val id = shown.layerId
        val list = if (lastList != null && (id === lastId || id == lastId)) lastList
            else lists.getOrPut(id) { ArrayList() }.also { lastId = id; lastList = it }
        list.add(one)
        val box = shown.bounds
        if (!(box.x.isFinite() && box.y.isFinite() && box.width.isFinite() && box.height.isFinite()) || box.width < 0 || box.height < 0) judged = false
        else { left = minOf(left, box.x); top = minOf(top, box.y); right = maxOf(right, box.x + box.width); bottom = maxOf(bottom, box.y + box.height) }
    }
    val order = ArrayList<String>(lists.size)
    for (id in layerIds) if (lists.containsKey(id)) order.add(id)
    if (order.size < lists.size) { val listed = layerIds.toHashSet(); for (id in lists.keys) if (id !in listed) order.add(id) }
    val bounds = if (!judged) null else if (objects.isEmpty()) Rect(0.0, 0.0, 0.0, 0.0) else Rect(left, top, right - left, bottom - top)
    // Where each layer's objects lie together, the runs are kept, and the next list is followed from them.
    val runs = ArrayList<LayerRun>(lists.size)
    var runStart = 0
    for (index in 1..objects.size) {
        if (index == objects.size || item(objects[index]).layerId != item(objects[runStart]).layerId) { pushRun(runs, item(objects[runStart]).layerId, runStart, index); runStart = index }
    }
    return LayerLists(order, lists, bounds, if (runs.size == lists.size) runs else null)
}

/** The first object of [list] that is drawn (not in [hidden]): it says how its layer is composited. */
internal fun <T> firstDrawn(list: List<T>, hidden: Set<String>, item: (T) -> RenderItem): T? {
    for (index in list.indices) if (item(list[index]).objectId !in hidden) return list[index]
    return null
}

/** One kept picture: the layer [layers] alone, or, with [prefix], the source
 * pixels and the layers [layers] composited as the document layer holds them. */
internal data class SlotKey(val layers: List<String>, val prefix: Boolean)

/** Which pictures are kept for the layers that draw anything, [drawn], in
 * drawing order: one for each, when there are at most [max]; otherwise the
 * topmost max - 1 have one each and the rest share a first picture with the
 * source pixels. */
internal fun slotLayout(drawn: List<String>, max: Int): List<SlotKey> {
    if (drawn.size <= max) return drawn.map { SlotKey(listOf(it), false) }
    val cut = drawn.size - (max - 1)
    return listOf(SlotKey(drawn.subList(0, cut).toList(), true)) + drawn.subList(cut, drawn.size).map { SlotKey(listOf(it), false) }
}

/** What happened to one layer's saved objects between a picture and a scene. */
internal sealed interface LayerStep {
    /** Each object draws as it did (its layer may be composited otherwise). */
    data object Same : LayerStep
    /** The objects of the scene's list from [from] on are new and follow all
     * the picture's: they are drawn into the picture, and nothing else is. */
    data class Append(val from: Int) : LayerStep
    /** Some objects went or came ([diff]: positions in the two lists). */
    data class Changed(val diff: ObjectsDiff) : LayerStep
    /** More changed than is followed one by one: the picture is drawn again. */
    data object Redraw : LayerStep
}

/** How [next] differs from [held], both one layer's saved objects in drawing
 * order. Objects that draw the same ([sameDrawing]) count as the same. */
internal fun <T : Any> layerStep(held: List<T>, next: List<T>, limit: Int, item: (T) -> RenderItem): LayerStep {
    if (held === next) return LayerStep.Same
    val diff = objectsDiff(held, next, limit) { a, b -> sameDrawing(item(a), item(b)) } ?: return LayerStep.Redraw
    if (diff.isEmpty) return LayerStep.Same
    if (diff.appended(held.size, next.size)) return LayerStep.Append(held.size)
    return LayerStep.Changed(diff)
}

/** What one kept picture needs to show a scene. */
internal enum class SlotWork { Keep, Append, Patch, Render }

/** What the picture [key] needs, given the pictures held ([held]), what
 * happened to each layer ([steps]; a layer missing there is [LayerStep.Same])
 * and whether the source pixels changed. A picture of one layer stands for a
 * change in how the layer is composited; one that holds several layers does
 * not, nor for a change of the source pixels, and takes strokes added to any
 * of them as a part drawn again. */
internal fun slotWork(key: SlotKey, held: Set<SlotKey>, steps: Map<String, LayerStep>, sourceChanged: Boolean, repainted: Set<String>): SlotWork {
    if (key !in held) return SlotWork.Render
    if (key.prefix && (sourceChanged || key.layers.any { it in repainted })) return SlotWork.Render
    var work = SlotWork.Keep
    for (layer in key.layers) when (val step = steps[layer] ?: LayerStep.Same) {
        LayerStep.Same -> Unit
        LayerStep.Redraw -> return SlotWork.Render
        is LayerStep.Changed -> work = SlotWork.Patch
        is LayerStep.Append -> if (key.prefix) work = SlotWork.Patch else if (work == SlotWork.Keep) work = SlotWork.Append
    }
    return work
}

// Views.

/** How the view a picture was drawn at and the view of a frame lie to each other. */
internal sealed interface ViewMove {
    data object Same : ViewMove
    /** Moved by whole pixels. [exact]: the page and every saved object lie
     * on the screen whole both times ([contentOnScreen]), so the picture moved
     * is what is drawn there, pixel for pixel (the renderer draws a path that
     * the screen's edge cuts otherwise than one that lies within, but draws a
     * path that lies within alike wherever it lies). */
    data class Shift(val dx: Int, val dy: Int, val exact: Boolean) : ViewMove
    /** Scaled, turned, or moved by part of a pixel: the picture is drawn under
     * [toScreen] (its pixels to the screen), softer, until it is drawn again. */
    data class Warp(val toScreen: Transform) : ViewMove
}

/** Moves within this much of whole pixels count as whole. */
internal const val SHIFT_TOLERANCE_PX = 1e-4

/** How a picture drawn at [made] (document to screen) lies in a frame at [now];
 * [madeWhole] and [nowWhole]: the page lies on the screen whole at each. Null
 * when [made] cannot be inverted. */
internal fun viewMove(made: Transform, now: Transform, madeWhole: Boolean, nowWhole: Boolean): ViewMove? {
    if (made == now) return ViewMove.Same
    if (made.a == now.a && made.b == now.b && made.c == now.c && made.d == now.d) {
        val dx = now.e - made.e; val dy = now.f - made.f
        val x = round(dx); val y = round(dy)
        if (abs(dx - x) <= SHIFT_TOLERANCE_PX && abs(dy - y) <= SHIFT_TOLERANCE_PX && abs(x) < 1e7 && abs(y) < 1e7) {
            if (x == 0.0 && y == 0.0) return ViewMove.Same
            return ViewMove.Shift(x.toInt(), y.toInt(), madeWhole && nowWhole)
        }
    }
    val back = invert(made) ?: return null
    return ViewMove.Warp(EditorDrawing.compose(now, back))
}

/** A pan of [dx], [dy] screen pixels, with [rest] carried from the moves
 * before it: the whole pixels the page is moved by now, and what is carried
 * on. The page then only ever moves by whole pixels, and kept pictures moved
 * with it are what is drawn there. */
internal fun wholePixelPan(rest: Point, dx: Double, dy: Double): Pair<Point, Point> {
    val x = rest.x + dx; val y = rest.y + dy
    val stepX = round(x); val stepY = round(y)
    return Point(stepX, stepY) to Point(x - stepX, y - stepY)
}

/** Whether the page of [pageWidth] x [pageHeight] lies whole on a [width] x [height] screen under [matrix]. */
internal fun pageOnScreen(matrix: Transform, pageWidth: Double, pageHeight: Double, width: Int, height: Int): Boolean =
    onScreen(matrix, Rect(0.0, 0.0, pageWidth, pageHeight), width, height)

/** Whether the page and every saved object ([content]: the box around all
 * of them; null when it cannot be judged) lie whole on the screen under
 * [matrix]: then nothing a picture holds is cut by the screen's edge. */
internal fun contentOnScreen(matrix: Transform, pageWidth: Double, pageHeight: Double, content: Rect?, width: Int, height: Int): Boolean =
    content != null && pageOnScreen(matrix, pageWidth, pageHeight, width, height) && onScreen(matrix, content, width, height)

private fun onScreen(matrix: Transform, box: Rect, width: Int, height: Int): Boolean {
    for (corner in 0 until 4) {
        val x = box.x + if (corner == 1 || corner == 2) box.width else 0.0
        val y = box.y + if (corner >= 2) box.height else 0.0
        val sx = matrix.a * x + matrix.c * y + matrix.e; val sy = matrix.b * x + matrix.d * y + matrix.f
        if (!(sx >= 0.0 && sx <= width && sy >= 0.0 && sy <= height)) return false
    }
    return true
}

/** The four screen corners of a [width] x [height] picture under [toScreen] (its pixels to the screen). */
internal fun pictureCorners(toScreen: Transform, width: Int, height: Int): DoubleArray {
    val corners = DoubleArray(8)
    for (corner in 0 until 4) {
        val x = if (corner == 1 || corner == 2) width.toDouble() else 0.0
        val y = if (corner >= 2) height.toDouble() else 0.0
        corners[corner * 2] = toScreen.a * x + toScreen.c * y + toScreen.e
        corners[corner * 2 + 1] = toScreen.b * x + toScreen.d * y + toScreen.f
    }
    return corners
}

/** The scale the page's soft picture is drawn at for a [width] x [height]
 * screen: the page fitted to the screen, so that the picture is no larger
 * than the screen; null for a page that cannot be judged. */
internal fun overviewScale(pageWidth: Double, pageHeight: Double, width: Int, height: Int): Double? {
    if (!(pageWidth > 0 && pageHeight > 0) || !pageWidth.isFinite() || !pageHeight.isFinite() || width <= 0 || height <= 0) return null
    val scale = minOf(width / pageWidth, height / pageHeight)
    return if (scale.isFinite() && scale > 0) scale else null
}

/** The picture a frame's view needs it to be drawn at again, once the view
 * rests: a pinch, a turn or a part-pixel move, or a whole-pixel move when the
 * screen's edge cuts the page. */
internal fun softView(move: ViewMove): Boolean = move is ViewMove.Warp || (move is ViewMove.Shift && !move.exact)

/** Why a frame is drawn as the whole scene, with no kept picture. The first
 * group are scenes a picture is not kept for at all, and one that is held is
 * given back; the second group last for a gesture or a moment, and a picture
 * that is held stays for the frames after it. */
internal enum class PictureRefusal(val keepsPicture: Boolean) {
    /** Not a hardware canvas: nothing here can be drawn into it. */
    Software(false),
    /** A tiled giant image, whose display memory is reserved and counted elsewhere. */
    Giant(false),
    /** A paged document, drawn from tiles only its reader may hold. */
    DocumentTiles(false),
    /** AI results, or results whose pixels are still being read. */
    Results(false),
    /** The surface is being resized in place; this generation may be about to leave. */
    Resizing(false),
    /** The frame is for another size than the scene was laid out for. */
    Size(true),
    /** A region is being marked on a document. */
    Draft(true),
    /** Saved objects are being moved or turned, or hidden while they are
     * erased, and what that changes cannot be drawn over the pictures: none is
     * held, or the change covers too much of the screen or cannot be placed. */
    Gesture(true),
    /** Ink in progress that cannot be drawn in its layer's place over the
     * pictures: the view is moving, or the ink is in a layer that shares a
     * picture with others above it. */
    Ink(true),
}

/** Null when a frame may be drawn from kept pictures. [width] and [height]
 * are the view the scene was laid out for, [canvasWidth] and [canvasHeight]
 * the frame being drawn. */
internal fun pictureRefusal(hardware: Boolean, canvasWidth: Int, canvasHeight: Int, width: Int, height: Int, resizing: Boolean,
    giant: Boolean, documentTiles: Boolean, resultsReady: Boolean, results: Boolean, draft: Boolean): PictureRefusal? = when {
    !hardware -> PictureRefusal.Software
    giant -> PictureRefusal.Giant
    documentTiles -> PictureRefusal.DocumentTiles
    !resultsReady || results -> PictureRefusal.Results
    resizing -> PictureRefusal.Resizing
    width <= 0 || height <= 0 || width != canvasWidth || height != canvasHeight -> PictureRefusal.Size
    draft -> PictureRefusal.Draft
    else -> null
}

/** Everything the pixels of the kept pictures depend on, besides their view:
 * the saved document as it is stacked, the source pixels and every object
 * layer, transparent elsewhere. The surround, the white page, the page's
 * edge, ink in progress, selection, hover and the magnifier are drawn over or
 * under them for each frame. Pictures are only ever made from a scene with no
 * gesture in it.
 *
 * [objects] and [background] are compared by identity: a saved object is
 * never changed in place, and an edit, an undo, an erasure, a move, a change
 * from the paired computer and another document or page all arrive as other
 * objects. */
internal class PictureBasis<T>(val width: Int, val height: Int, val matrix: Transform, val scale: Double,
    val background: Any?, val smoothBackground: Boolean, val layerIds: List<String>, val hidden: Set<String>, val objects: List<T>)

/** What made the kept pictures wrong as a whole, or one of them, so that it is drawn again. */
internal enum class PictureChange {
    Missing, Size, View, Background, Layers,
    /** The saved objects changed in a way that cannot be drawn by parts: an
     * object whose drawing its box does not hold, or more objects than are
     * followed one by one. */
    Objects,
    /** What changed covers more than half of the screen. */
    Area,
}

/** Whether a picture that has to be drawn again ([why]) is drawn for this
 * frame. It is, unless the scene is changing from frame to frame, as it does
 * while the paired computer moves an object: then each frame would make a
 * picture that the next one throws away. Such frames are drawn whole, and the
 * picture is made once the scene has stopped changing. A change of size is
 * taken to be the start of such a movement; any other change is made at once
 * unless it follows another closely. [sameAsLastFrame]: the frame before
 * wanted this very picture. [sinceChangeMs]: how long ago a frame last wanted
 * another picture than the one before it, negative when none ever did. */
internal fun pictureMadeNow(sameAsLastFrame: Boolean, sinceChangeMs: Long, why: PictureChange): Boolean =
    sameAsLastFrame || (why != PictureChange.View && why != PictureChange.Size && (sinceChangeMs < 0 || sinceChangeMs >= PICTURE_STREAK_MS))

/** Changes closer together than this are one movement. */
internal const val PICTURE_STREAK_MS = 100L

/** Whether pictures drawn at a view that has to be drawn again once it rests
 * are drawn at it right after this frame: when the gesture that moved it
 * ended ([moving] went false after [wasMoving]), or when the view changed
 * once, not as part of a movement ([sinceViewChangeMs] at least
 * [PICTURE_STREAK_MS] or never). Otherwise once no frame has come for a while. */
internal fun viewSettlesNow(moving: Boolean, wasMoving: Boolean, sinceViewChangeMs: Long): Boolean =
    !moving && (wasMoving || sinceViewChangeMs < 0 || sinceViewChangeMs >= PICTURE_STREAK_MS)

/** How often each thing happened, for the on-device tests and nothing else. */
internal class PictureCounts {
    /** Every picture drawn from the saved objects (first, a new size, a view
     * that rested after it moved), a picture a stroke was added to, a part
     * drawn again, pictures moved by whole pixels, one layer's picture drawn
     * from its objects; frames drawn from pictures, frames with a part drawn
     * again over them, frames drawn from pictures moved, scaled or turned,
     * frames drawn as a whole scene, and renders that failed. */
    val rebuilt = AtomicLong(); val added = AtomicLong(); val patched = AtomicLong(); val shifted = AtomicLong(); val layers = AtomicLong()
    val frames = AtomicLong(); val over = AtomicLong(); val softFrames = AtomicLong(); val whole = AtomicLong(); val failed = AtomicLong()
    /** The page's soft picture: made from the saved objects, and drawn again in part. */
    val overviews = AtomicLong(); val overviewPatches = AtomicLong()
    /** Pictures that came out only once their surface was set again ([PictureSurface.render]). */
    val retried = AtomicLong()
    /** The pixels of the part last drawn again, into a picture or over it. */
    val area = AtomicLong()
    /** The last frame drawn was drawn from pictures moved, scaled or turned,
     * or filled from the page's soft picture: not yet the whole scene. */
    val soft = AtomicBoolean()
    /** Frames drawn whole, by [PictureRefusal]. */
    val refused = AtomicLongArray(PictureRefusal.entries.size)
    override fun toString(): String = "rebuilt=${rebuilt.get()} added=${added.get()} patched=${patched.get()} shifted=${shifted.get()} " +
        "layers=${layers.get()} frames=${frames.get()} over=${over.get()} soft_frames=${softFrames.get()} whole=${whole.get()} " +
        "failed=${failed.get()} overviews=${overviews.get()}/${overviewPatches.get()} retried=${retried.get()} area=${area.get()} soft=${soft.get()}" +
        PictureRefusal.entries.joinToString("") { reason -> refused.get(reason.ordinal).let { if (it == 0L) "" else " ${reason.name.lowercase()}=$it" } }
}

/** On-device tests only: one frame as the kept pictures draw it and the same
 * scene drawn whole, both as plain pixels, and where the page lies in them
 * ([matrix] maps the page of [pageWidth] x [pageHeight] to the frame). */
internal class PictureComparison(val kept: Bitmap, val whole: Bitmap, val matrix: Transform, val pageWidth: Double, val pageHeight: Double)

/** One group of the document stack as a frame composites it: the kept
 * picture of [layers] ([prefix]: the source pixels and those layers, as the
 * document layer holds them; null: none, [extra] is all there is), composited
 * as [paint] says, and saved objects not yet in it and ink in progress
 * ([extra]) drawn over it in the same group. */
internal class FrameGroup(val layers: List<String>, val prefix: Boolean, val picture: Bitmap?, val paint: RenderItem?, val extra: List<DrawObject>)

/** What fills the part of the page a frame's moved pictures do not cover. */
internal sealed interface FrameFill {
    /** The page's soft picture, under [toScreen] (its pixels to the screen). */
    class Overview(val bitmap: Bitmap, val toScreen: Transform) : FrameFill
    /** The saved objects that can reach the screen, drawn. */
    class Objects(val objects: List<DrawObject>) : FrameFill
}

/** How one frame is drawn from kept pictures ([EditorDrawing.drawFrame]):
 * [groups] in drawing order, the source pixels first when no picture holds
 * them ([source]); the pictures [offsetX], [offsetY] whole pixels from where
 * they were drawn, or under [toScreen] when it is given; [cover] the part of
 * the screen they hold when they do not hold all of it, the rest filled from
 * [fill]; the boxes [boxes] drawn from [parts]; layers composited as [paints]
 * says; the selection outline of [selection]. */
internal class FramePlan(val groups: List<FrameGroup>, val source: Boolean, val offsetX: Int, val offsetY: Int, val toScreen: Transform?,
    val cover: Path?, val fill: FrameFill?, val boxes: List<PixelBox>, val parts: List<List<DrawObject>>,
    val paints: Map<String, RenderItem>, val selection: DrawObject?)

/** The kept pictures of one render session, and the frames drawn from them.
 *
 * [draw] draws a whole frame when the scene allows it: the surround, the page,
 * the pictures composited, the ink in progress in its layer, the overlays and
 * the magnifier. Strokes just saved are drawn over their layer's picture in
 * that frame, and into it by a message right behind it; a change that touched
 * a part of the page is drawn over the pictures in that frame and into the
 * pictures of the layers it changed right behind it, so that no frame waits
 * for a picture to be drawn off screen. A picture that has to be drawn whole
 * (a layer shown, or whose change cannot be placed) is drawn first. For a
 * scene with a gesture in it (objects being moved or turned, objects hidden
 * while erased) it leaves the pictures as they are and draws, over them, the
 * part of the screen the gesture changes. When it returns false nothing was
 * drawn and the caller draws the whole scene as it always could.
 *
 * While the view moves, frames draw the pictures moved, scaled or turned
 * ([ViewMove]) and fill what they leave uncovered from the page's soft
 * picture (the overview, made when the canvas is idle and kept up to date in
 * part), or from the saved objects while it is not up to date. Once the view
 * rests (no frame for [PICTURE_SETTLE_MS], or at once when the gesture ends:
 * [CanvasScene.viewMoving]) the pictures are drawn at it and [settled] is
 * called, so that a frame shows the whole scene again. A whole-pixel pan with
 * the page on the screen whole needs no drawing: the pictures are only moved.
 *
 * A part of the screen is drawn from the saved objects a [SavedIndex] finds
 * there, and the pen's loupe finds them the same way: no part of a frame
 * visits every saved object. The index follows the saved objects as they
 * change, by the objects that changed ([objectsDiff]).
 *
 * Memory: at rest one screen-sized picture for each layer that draws, at most
 * [MAX_SLOTS] (beyond that the lower layers share one with the source
 * pixels), and the overview, no larger than the screen. While pictures change,
 * the new ones are drawn while the old are held until the frames showing them
 * are out, at most [SLOT_PICTURES] of them; [REST_MS] after the last change
 * the renderers and their spare buffers are given up. Everything goes at
 * [forget] and [discard], and nothing is made after [discard].
 *
 * Owned by one render session. [draw] and [compare] run on its drawing
 * thread, and so does everything this posts to that thread. [requestDrop] may
 * be called from any thread. [discard] runs on the main thread once the
 * session has been released; a message of this class still waiting on the
 * drawing thread may run beside it, so every entry holds this object's lock,
 * which nothing else ever contends for. */
internal class KeptPicture(val counts: PictureCounts = PictureCounts(), private val settled: (() -> Unit)? = null) {
    private class Slot(val key: SlotKey, val picture: PictureSurface.Picture)
    private class Held(val slots: List<Slot>, val basis: PictureBasis<DrawObject>, val lists: LayerLists<DrawObject>)
    private class Overview(val picture: PictureSurface.Picture, val scale: Double, val objects: List<DrawObject>, val hidden: Set<String>,
                           val layerIds: List<String>, val paints: List<Any>, val background: Any?, val smooth: Boolean, val pageWidth: Double, val pageHeight: Double)

    /** What the pictures need to show a scene with no gesture in it: what
     * each layer's objects did, the pictures' work, and the part drawn again. */
    private class Plan(val move: ViewMove, val lists: LayerLists<DrawObject>, val layout: List<SlotKey>, val work: Map<SlotKey, SlotWork>,
                       val steps: Map<String, LayerStep>, val area: TouchedArea?, val paints: Map<String, RenderItem>, val why: PictureChange?)

    private var surface: PictureSurface? = null
    private var overviewSurface: PictureSurface? = null
    private var held: Held? = null
    private var overview: Overview? = null
    /** The saved objects of the scene last drawn, found by place and by id. */
    private var index: SavedIndex<DrawObject>? = null
    private var diffFrom: List<DrawObject>? = null
    private var diffTo: List<DrawObject>? = null
    private var diffAnswer: ObjectsDiff? = null
    private fun diffOf(from: List<DrawObject>, to: List<DrawObject>): ObjectsDiff? {
        if (from === to) return ObjectsDiff.NONE
        if (from !== diffFrom || to !== diffTo) { diffAnswer = objectsDiff(from, to, MAX_CHANGED); diffFrom = from; diffTo = to }
        return diffAnswer
    }
    /** Layer lists, worked out once for each list of saved objects. */
    private var listsFrom: List<DrawObject>? = null
    private var listsIds: List<String>? = null
    private var listsAnswer: LayerLists<DrawObject>? = null
    private fun listsOf(objects: List<DrawObject>, ids: List<String>): LayerLists<DrawObject> {
        held?.let { if (it.basis.objects === objects && it.basis.layerIds == ids) return it.lists }
        if (objects !== listsFrom || ids != listsIds) {
            val now = held
            listsAnswer = (if (now != null && now.basis.layerIds == ids) followLayerLists(now.lists, now.basis.objects, objects, ids, MAX_CHANGED) { it.item } else null)
                ?: layerLists(ids, objects) { it.item }
            listsFrom = objects; listsIds = ids
        }
        return checkNotNull(listsAnswer)
    }

    /** The last plan, for the same held pictures and scene. */
    private var planHeld: Held? = null
    private var planScene: CanvasScene? = null
    private var planAnswer: Plan? = null
    private var layerIdsOf: DocumentSnapshot? = null
    private var layerIds: List<String> = emptyList()
    private var failures = 0
    private var discarded = false
    private var lastPlan: FramePlan? = null
    /** Pictures replaced and not yet released. */
    private val replaced = ArrayList<PictureSurface.Picture>(4)
    /** The scene the last frame wanted the pictures brought to, and when a
     * frame last wanted another than the one before it. */
    private var wanted: PictureBasis<DrawObject>? = null
    private var wantedScene: CanvasScene? = null
    private var changedMs = 0L
    /** The last attempt to bring the pictures up to date found no room for the new ones. */
    private var roomShort = false
    /** The view of the frame before, whether a gesture was moving it, and when it last changed. */
    private var lastMatrix: Transform? = null
    private var lastMoving = false
    private var viewChangedMs = 0L
    /** The newest scene with no gesture in it, for the overview. */
    private var restingScene: CanvasScene? = null
    @Volatile private var handler: Handler? = null
    @Volatile private var dropAsked = false
    private val rest = Runnable { synchronized(this) { surface?.close(); surface = null; overviewSurface?.close(); overviewSurface = null } }
    private val drop = Runnable { synchronized(this) { if (dropAsked) { dropAsked = false; forget() } } }
    /** Right after a frame drawn over the pictures, or once the view or a
     * movement has come to rest: the pictures are brought up to date for the
     * next frame, and when the frame on screen was not the whole scene, one
     * that is is asked for. */
    private val settle: Runnable = Runnable {
        synchronized(this) {
            val basis = wanted; val scene = wantedScene; val post = handler
            wanted = null; wantedScene = null
            if (!discarded && basis != null && scene != null && post != null && failures < MAX_FAILURES) {
                val wasSoft = counts.soft.get()
                val index = indexFor(scene)
                roomShort = false
                if (bring(post, scene, basis, index)) { if (wasSoft) settled?.invoke() }
                else if (roomShort && wanted == null) {
                    // Pictures replaced a moment ago are not given back yet: tried again shortly.
                    wanted = basis; wantedScene = scene; post.postDelayed(settle, PICTURE_SETTLE_MS)
                }
            }
        }
    }
    private val idle = Runnable { synchronized(this) { val post = handler; val scene = restingScene; if (!discarded && post != null && scene != null) updateOverview(post, scene) } }

    /** [steady]: no saved object is hidden for an erasure in progress.
     * [resizing]: the session owes the redraw of an in-place resize.
     * [first]: the session has not shown a frame yet. Its first frame is drawn
     * whole, so that a new canvas appears no later than it used to; the
     * pictures are made right after, before a pen can need them. */
    @Synchronized fun draw(canvas: Canvas, scene: CanvasScene, steady: Boolean, resizing: Boolean, first: Boolean): Boolean {
        lastPlan = null
        if (discarded) return false
        if (dropAsked) { dropAsked = false; forget() }
        val width = scene.camera.viewportWidth.toInt(); val height = scene.camera.viewportHeight.toInt()
        val refusal = pictureRefusal(canvas.isHardwareAccelerated, canvas.width, canvas.height, width, height, resizing,
            giant = scene.giantMode || scene.giantFrame != null, documentTiles = scene.documentFrame != null,
            resultsReady = scene.resultDisplayReady, results = scene.aiResults.isNotEmpty(), draft = scene.documentDraft != null)
        if (refusal != null) return refused(refusal)
        val looper = Looper.myLooper()
        if (looper == null || failures >= MAX_FAILURES) { counts.whole.incrementAndGet(); counts.soft.set(false); return false }
        val post = handler?.takeIf { it.looper === looper } ?: Handler(looper).also { handler = it }
        val uptime = SystemClock.uptimeMillis()
        // The view: when it last changed, and whether a gesture is moving it.
        val wasMoving = lastMoving
        // How long before this frame the view last changed: a view that changes now after resting is a single change.
        val sinceViewChange = if (viewChangedMs == 0L) -1 else uptime - viewChangedMs
        if (lastMatrix != null && lastMatrix != scene.matrix) viewChangedMs = uptime
        lastMatrix = scene.matrix; lastMoving = scene.viewMoving
        val ids = if (layerIdsOf === scene.document) layerIds else scene.document.layers.map { it.id }.also { layerIds = it; layerIdsOf = scene.document }
        val index = indexFor(scene)
        val next = PictureBasis(width, height, scene.matrix, scene.camera.scale, scene.background, scene.smoothBackground, ids, scene.hidden, scene.objects)
        val gesture = !steady || scene.transformed.isNotEmpty()
        if (!gesture) restingScene = scene
        val now = held
        if (now == null || now.basis.width != width || now.basis.height != height) {
            // Nothing to draw from: the pictures are made for this frame, or,
            // for a new canvas, a new size or a frame among a streak of
            // changes, once the scene rests, and till then frames are whole.
            val why = if (now == null) PictureChange.Missing else PictureChange.Size
            if (gesture || first || !madeNow(next, uptime, why)) return wholeUntilSettled(post, scene, next, first, gesture)
            post.removeCallbacks(settle); wanted = null; wantedScene = null
            if (!renderAll(post, scene, next, index)) return wholeFrame()
            return drawPlanned(canvas, scene, index, post, uptime, wasMoving, sinceViewChange, gesture = false)
        }
        return drawPlanned(canvas, scene, index, post, uptime, wasMoving, sinceViewChange, gesture)
    }

    /** A frame from the pictures held: the plan for [scene], any picture it
     * needs drawn first, the frame, and the work left for after it. */
    private fun drawPlanned(canvas: Canvas, scene: CanvasScene, index: SavedIndex<DrawObject>, post: Handler, uptime: Long, wasMoving: Boolean,
                            sinceViewChange: Long, gesture: Boolean): Boolean {
        var now = checkNotNull(held)
        val next = PictureBasis(now.basis.width, now.basis.height, scene.matrix, scene.camera.scale, scene.background, scene.smoothBackground,
            layerIds, scene.hidden, scene.objects)
        var plan = planFor(now, scene, next, index) ?: return refused(PictureRefusal.Gesture)
        val soft = softView(plan.move)
        if (!gesture && !soft) {
            // Pictures that have to be drawn whole are drawn first; while the
            // scene changes from frame to frame, the frame is drawn whole. A
            // small layer (one shown, a highlighter's) is drawn into the frame
            // from its objects instead, and its picture made right after.
            val renders = plan.work.filterValues { it == SlotWork.Render }.keys
            if (renders.any { !smallLayer(it, plan) }) {
                val why = plan.why ?: PictureChange.Layers
                if (!madeNow(next, uptime, why)) return wholeUntilSettled(post, scene, next, first = false, gesture = false)
                post.removeCallbacks(settle); wanted = null; wantedScene = null
                if (!bring(post, scene, next, index)) return wholeFrame()
                now = checkNotNull(held)
                plan = planFor(now, scene, next, index) ?: return wholeFrame()
            }
        }
        val frame = framePlan(now, scene, plan, index, gesture, soft) ?: return refused(if (scene.provisional.isNotEmpty()) PictureRefusal.Ink else PictureRefusal.Gesture)
        EditorDrawing.drawFrame(canvas, scene, frame)
        lastPlan = frame
        val moving = movingObjects(scene, index)
        PrecisionDrawing.drawPanes(canvas, scene, onlyVisible = true) { visible, margin -> index.near(visible, margin, moving) }
        counts.frames.incrementAndGet()
        if (frame.boxes.isNotEmpty()) { counts.over.incrementAndGet(); counts.area.set(frame.boxes.sumOf { it.pixels }) }
        val filled = frame.cover != null || frame.toScreen != null
        if (filled) counts.softFrames.incrementAndGet()
        counts.soft.set(filled)
        if (!gesture) afterFrame(post, scene, next, plan, uptime, wasMoving, sinceViewChange)
        post.removeCallbacks(idle); post.postDelayed(idle, OVERVIEW_IDLE_MS)
        return true
    }

    /** What is left to do once a frame of a scene with no gesture in it is out. */
    private fun afterFrame(post: Handler, scene: CanvasScene, next: PictureBasis<DrawObject>, plan: Plan, uptime: Long, wasMoving: Boolean,
                           sinceViewChange: Long) {
        val now = checkNotNull(held)
        val pending = plan.move != ViewMove.Same || plan.work.any { it.value != SlotWork.Keep } ||
            plan.layout != now.slots.map { it.key }
        if (!pending) {
            // Nothing to draw: the pictures are the scene's, which is taken as
            // theirs (other instances of the same objects, layers otherwise composited).
            post.removeCallbacks(settle); wanted = null; wantedScene = null
            if (now.basis.objects !== next.objects || now.basis.hidden !== next.hidden || now.basis.background !== next.background || now.basis.layerIds != next.layerIds)
                held = Held(now.slots, next, plan.lists)
            return
        }
        val same = wanted?.let { sameScene(it, next) } == true
        if (!same) changedMs = uptime
        wanted = next; wantedScene = scene
        post.removeCallbacks(settle)
        when {
            softView(plan.move) || plan.move is ViewMove.Shift ->
                if (viewSettlesNow(scene.viewMoving, wasMoving, sinceViewChange)) post.post(settle)
                else post.postDelayed(settle, PICTURE_SETTLE_MS)
            plan.work.any { it.value == SlotWork.Patch } ->
                // Patches made from frame to frame (the paired computer moving an object) wait for the movement to rest.
                if (pictureMadeNow(same, if (changedMs == 0L) -1 else uptime - changedMs, PictureChange.Objects)) post.post(settle)
                else post.postDelayed(settle, PICTURE_SETTLE_MS)
            else -> post.post(settle)
        }
    }

    private fun sameScene(a: PictureBasis<DrawObject>, b: PictureBasis<DrawObject>): Boolean =
        a.objects === b.objects && (a.hidden === b.hidden || a.hidden == b.hidden) && a.matrix == b.matrix && a.background === b.background &&
            a.layerIds == b.layerIds && a.width == b.width && a.height == b.height

    /** Whether a picture that has to be drawn whole for [next] ([why]) is drawn now. */
    private fun madeNow(next: PictureBasis<DrawObject>, uptime: Long, why: PictureChange): Boolean {
        val same = wanted?.let { sameScene(it, next) } == true
        val made = pictureMadeNow(same, if (changedMs == 0L) -1 else uptime - changedMs, why)
        if (!same) changedMs = uptime
        return made
    }

    /** A frame drawn whole; the pictures are made for [next] when no frame has come for a moment, or right after a first frame. */
    private fun wholeUntilSettled(post: Handler, scene: CanvasScene, next: PictureBasis<DrawObject>, first: Boolean, gesture: Boolean): Boolean {
        // A picture is never made from a scene with a gesture in it.
        if (!gesture) {
            wanted = next; wantedScene = scene
            post.removeCallbacks(settle)
            if (first) post.post(settle) else post.postDelayed(settle, PICTURE_SETTLE_MS)
        }
        return wholeFrame()
    }

    private fun wholeFrame(): Boolean { counts.whole.incrementAndGet(); counts.soft.set(false); return false }

    /** The plan for [scene] from [now], worked out once for each pair. Null:
     * the view cannot be judged. */
    private fun planFor(now: Held, scene: CanvasScene, next: PictureBasis<DrawObject>, index: SavedIndex<DrawObject>): Plan? {
        if (planHeld === now && planScene?.let { sameFor(it, scene) } == true) return planAnswer
        val pageWidth = scene.document.width.toDouble(); val pageHeight = scene.document.height.toDouble()
        val lists = listsOf(next.objects, next.layerIds)
        // Moved by whole pixels, a picture is the scene drawn there only when nothing in it is cut by the screen's edge, before or after.
        val move = viewMove(now.basis.matrix, next.matrix, contentOnScreen(now.basis.matrix, pageWidth, pageHeight, now.lists.bounds, next.width, next.height),
            contentOnScreen(next.matrix, pageWidth, pageHeight, lists.bounds, next.width, next.height)) ?: return null
        val steps = HashMap<String, LayerStep>()
        var why: PictureChange? = null
        val sameObjects = now.basis.objects === next.objects
        if (!sameObjects) for (layer in now.lists.layers + lists.layers) {
            val step = layerStep(now.lists.of(layer), lists.of(layer), MAX_CHANGED) { it.item }
            if (step != LayerStep.Same) steps[layer] = step
        }
        // A layer's picture holds its drawn objects: one that hides or shows objects for good is drawn by parts.
        val hiddenChanged = !(now.basis.hidden === next.hidden || now.basis.hidden == next.hidden)
        val drawn = lists.order.filter { firstDrawn(lists.of(it), next.hidden) { one -> one.item } != null }
        val layout = slotLayout(drawn, MAX_SLOTS)
        val paints = HashMap<String, RenderItem>()
        for (layer in lists.order) firstDrawn(lists.of(layer), next.hidden) { it.item }?.let { paints[layer] = it.item }
        val heldPaints = HashMap<String, RenderItem>()
        for (layer in now.lists.order) firstDrawn(now.lists.of(layer), now.basis.hidden) { it.item }?.let { heldPaints[layer] = it.item }
        val repainted = HashSet<String>()
        for ((layer, paint) in paints) { val before = heldPaints[layer]; if (before != null && !samePaint(before, paint)) repainted += layer }
        val sourceChanged = now.basis.background !== next.background || now.basis.smoothBackground != next.smoothBackground
        val heldKeys = now.slots.mapTo(HashSet()) { it.key }
        val work = HashMap<SlotKey, SlotWork>()
        for (key in layout) work[key] = slotWork(key, heldKeys, steps, sourceChanged, repainted)
        // Where the changed layers' objects were and are, and objects hidden or
        // shown for good: only for pictures drawn by parts. A layer hidden or
        // gone has no picture any more, and one shown is drawn whole.
        var area: TouchedArea? = TouchedArea(next.width, next.height)
        val placed = area!!
        val margin = if (softView(move)) SOFT_MARGIN_PX else 0.0
        val unplaced = HashSet<String>()
        val byParts = HashSet<String>()
        for (key in layout) if (work[key] == SlotWork.Patch || work[key] == SlotWork.Keep || work[key] == SlotWork.Append) byParts.addAll(key.layers)
        for ((layer, step) in steps) {
            if (step !is LayerStep.Changed && step !is LayerStep.Append || layer !in byParts) continue
            val heldList = now.lists.of(layer); val nextList = lists.of(layer)
            val diff = if (step is LayerStep.Changed) step.diff else null
            var fits = true
            if (diff != null) {
                for (at in diff.removed) if (!addDrawn(placed, heldList[at].item, next.matrix, null, margin)) { fits = false; break }
                if (fits) for (at in diff.added) if (!addDrawn(placed, nextList[at].item, next.matrix, null, margin)) { fits = false; break }
            } else if (layout.any { it.prefix && layer in it.layers }) {
                // Strokes added to a layer that shares a picture are drawn into it by parts.
                for (at in (step as LayerStep.Append).from until nextList.size) if (!addDrawn(placed, nextList[at].item, next.matrix, null, margin)) { fits = false; break }
            }
            if (!fits) unplaced += layer
        }
        if (hiddenChanged) {
            for (id in next.hidden) if (id !in now.basis.hidden) indexFind(id)?.takeIf { it.item.layerId in byParts }?.let {
                if (!addDrawn(placed, it.item, next.matrix, null, margin)) unplaced += it.item.layerId }
            for (id in now.basis.hidden) if (id !in next.hidden) indexFind(id)?.takeIf { it.item.layerId in byParts }?.let {
                if (!addDrawn(placed, it.item, next.matrix, null, margin)) unplaced += it.item.layerId }
        }
        // Grown until it cuts no object, so that what is drawn again is the whole scene's pixels.
        closeArea(placed, next.matrix, { box -> nearBox(scene, index, box) }, { it.item }, { null }, margin)
        if (unplaced.isNotEmpty()) {
            why = PictureChange.Objects
            for (key in layout) if (key.layers.any { it in unplaced }) work[key] = SlotWork.Render
        }
        if (placed.large) {
            // Drawing the part again would cost as much as drawing the layers it touched.
            why = why ?: PictureChange.Area
            for (key in layout) if (work[key] == SlotWork.Patch) work[key] = SlotWork.Render
            area = null
        } else if (hiddenChanged && !placed.isEmpty) {
            for (key in layout) if (work[key] == SlotWork.Keep && key.layers.any { layer -> lists.of(layer).any { it.item.objectId in next.hidden != it.item.objectId in now.basis.hidden } })
                work[key] = SlotWork.Patch
        }
        val answer = Plan(move, lists, layout, work, steps, area, paints, if (work.values.any { it == SlotWork.Render }) (why ?: PictureChange.Layers) else null)
        planHeld = now; planScene = scene; planAnswer = answer
        return answer
    }

    private fun sameFor(a: CanvasScene, b: CanvasScene): Boolean = a.objects === b.objects && a.matrix == b.matrix && a.hidden === b.hidden &&
        a.background === b.background && a.smoothBackground == b.smoothBackground && a.document === b.document &&
        a.camera.viewportWidth == b.camera.viewportWidth && a.camera.viewportHeight == b.camera.viewportHeight

    private fun indexFind(id: String): DrawObject? = index?.find(id)

    /** How the frame of [scene] is drawn from [now] by [plan]; null when it cannot be. */
    private fun framePlan(now: Held, scene: CanvasScene, plan: Plan, index: SavedIndex<DrawObject>, gesture: Boolean, soft: Boolean): FramePlan? {
        val width = now.basis.width; val height = now.basis.height
        val move = plan.move
        // Ink in progress is drawn in its layer's group, which needs the pictures at their place.
        if (scene.provisional.isNotEmpty() && soft) return null
        val pictures = HashMap<SlotKey, Bitmap>()
        for (slot in now.slots) pictures[slot.key] = slot.picture.bitmap
        // The boxes: the part changed objects touch, and what a gesture changes.
        val area = TouchedArea(width, height)
        val margin = if (soft) SOFT_MARGIN_PX else 0.0
        plan.area?.let { for (box in it.boxes) area.add(box.left.toDouble(), box.top.toDouble(), box.right.toDouble(), box.bottom.toDouble()) }
        if (plan.area == null && plan.work.values.any { it == SlotWork.Patch || it == SlotWork.Render } && !soft) return null
        if (gesture) {
            for ((id, preview) in scene.transformed) {
                val moved = index.find(id)?.item ?: continue
                if (preview == moved.transform) continue
                if (!addDrawn(area, moved, scene.matrix, null, margin) || !addDrawn(area, moved, scene.matrix, preview, margin)) return null
            }
            val restHidden = now.basis.hidden
            if (!(restHidden === scene.hidden || restHidden == scene.hidden)) {
                for (id in scene.hidden) if (id !in restHidden) { val gone = index.find(id)?.item ?: continue
                    if (!addDrawn(area, gone, scene.matrix, null, margin) || !addDrawn(area, gone, scene.matrix, scene.transformed[id], margin)) return null }
                for (id in restHidden) if (id !in scene.hidden) { val back = index.find(id)?.item ?: continue
                    if (!addDrawn(area, back, scene.matrix, null, margin) || !addDrawn(area, back, scene.matrix, scene.transformed[id], margin)) return null }
            }
            closeArea(area, scene.matrix, { box -> nearBox(scene, index, box) }, { it.item }, { scene.transformed[it.item.objectId] }, margin)
            if (area.large) return null
        }
        // The groups, in drawing order, with what each draws over its picture.
        val wet = HashMap<String, ArrayList<DrawObject>>()
        for (ink in scene.provisional) if (ink.item.objectId !in scene.hidden) wet.getOrPut(ink.item.layerId) { ArrayList() }.add(ink)
        val groups = ArrayList<FrameGroup>()
        val inSlot = HashMap<String, SlotKey>()
        for (key in plan.layout) for (layer in key.layers) inSlot[layer] = key
        val prefix = plan.layout.firstOrNull()?.takeIf { it.prefix }
        if (prefix != null) {
            if (prefix.layers.any { it in wet }) return null
            val picture = pictures[prefix]
            if (picture == null || plan.work[prefix] == SlotWork.Render) return null
            groups.add(FrameGroup(prefix.layers, true, picture, null, emptyList()))
        }
        val order = ArrayList<String>(plan.lists.order)
        for (layer in wet.keys) if (layer !in plan.lists.layers) order.add(layer)
        val listed = layerIds.toHashSet()
        // Ink in a layer nothing saved uses is drawn where the whole scene draws it: listed layers in their place, others last.
        val ordered = ArrayList<String>()
        for (id in layerIds) if (id in plan.lists.layers || id in wet) ordered.add(id)
        for (id in order) if (id !in listed && id !in ordered) ordered.add(id)
        var prefixDone = prefix == null
        for (layer in ordered) {
            val key = inSlot[layer]
            if (prefix != null && key == prefix) { prefixDone = true; continue }
            if (!prefixDone && layer in wet) return null
            val ink = wet[layer].orEmpty()
            if (key == null) {
                // A layer with nothing drawn saved: only ink, composited as its first ink says.
                if (ink.isNotEmpty()) groups.add(FrameGroup(listOf(layer), false, null, ink.first().item, ink))
                continue
            }
            val work = plan.work[key] ?: SlotWork.Keep
            val picture = pictures[key]
            val extra = ArrayList<DrawObject>()
            var drawnPicture = picture
            when {
                picture == null || work == SlotWork.Render -> {
                    // Not drawn yet at all: its objects are drawn here, all of them that can reach the screen.
                    if (!soft && !smallLayer(key, plan)) return null
                    drawnPicture = null
                    val own = plan.lists.of(layer)
                    extra.addAll(if (own.size <= SMALL_LAYER) visibleOfList(scene, own) else visibleOf(scene, index, setOf(layer)))
                }
                work == SlotWork.Append -> {
                    val step = plan.steps[layer]
                    if (step is LayerStep.Append) { val list = plan.lists.of(layer); for (at in step.from until list.size) if (list[at].item.objectId !in scene.hidden) extra.add(list[at]) }
                }
            }
            extra.addAll(ink)
            groups.add(FrameGroup(key.layers, false, drawnPicture, plan.paints[layer] ?: ink.firstOrNull()?.item, extra))
        }
        val parts = parts(scene, index, area.boxes, null)
        // Where the pictures lie on the screen, and what fills what they do not cover.
        var offsetX = 0; var offsetY = 0; var toScreen: Transform? = null; var cover: Path? = null; var fill: FrameFill? = null
        when (move) {
            ViewMove.Same -> Unit
            is ViewMove.Shift -> {
                offsetX = move.dx; offsetY = move.dy
                if (!move.exact) {
                    cover = Path().apply { addRect(move.dx.toFloat(), move.dy.toFloat(), (move.dx + width).toFloat(), (move.dy + height).toFloat(), Path.Direction.CW) }
                    fill = fillFor(scene, index, plan)
                }
            }
            is ViewMove.Warp -> {
                toScreen = move.toScreen
                val corners = pictureCorners(move.toScreen, width, height)
                cover = Path().apply {
                    moveTo(corners[0].toFloat(), corners[1].toFloat()); lineTo(corners[2].toFloat(), corners[3].toFloat())
                    lineTo(corners[4].toFloat(), corners[5].toFloat()); lineTo(corners[6].toFloat(), corners[7].toFloat()); close()
                }
                fill = fillFor(scene, index, plan)
            }
        }
        return FramePlan(groups, prefix == null, offsetX, offsetY, toScreen, cover, fill, area.boxes.toList(), parts, plan.paints,
            scene.selected?.let(index::find))
    }

    /** Whether the picture [key] holds one layer of at most [SMALL_LAYER] objects. */
    private fun smallLayer(key: SlotKey, plan: Plan): Boolean = !key.prefix && plan.lists.of(key.layers.single()).size <= SMALL_LAYER

    /** What fills the page where moved pictures do not reach: the overview
     * when it shows the scene's saved objects as they are, else the objects. */
    private fun fillFor(scene: CanvasScene, index: SavedIndex<DrawObject>, plan: Plan): FrameFill {
        val kept = overview
        if (kept != null && kept.objects === scene.objects && (kept.hidden === scene.hidden || kept.hidden == scene.hidden) &&
            kept.background === scene.background && kept.smooth == scene.smoothBackground && kept.layerIds == layerIds &&
            kept.paints == paintKeys(plan.lists, plan.paints) && kept.pageWidth == scene.document.width.toDouble() &&
            kept.pageHeight == scene.document.height.toDouble()) {
            return FrameFill.Overview(kept.picture.bitmap, EditorDrawing.compose(scene.matrix, Transform(a = 1 / kept.scale, d = 1 / kept.scale)))
        }
        return FrameFill.Objects(visibleOf(scene, index, null))
    }

    /** How each layer that draws is composited, in order: what the overview depends on besides the objects. */
    private fun paintKeys(lists: LayerLists<DrawObject>, paints: Map<String, RenderItem>): List<Any> =
        lists.order.mapNotNull { layer -> paints[layer]?.let { listOf(layer, layerAlpha(it), it.layerBlend == "multiply") } }

    /** Of [list] (one layer's saved objects in drawing order), those that can reach the screen, hidden ones left out. */
    private fun visibleOfList(scene: CanvasScene, list: List<DrawObject>): List<DrawObject> {
        val visible = visibleDocumentRect(scene.matrix, Rect(0.0, 0.0, scene.camera.viewportWidth, scene.camera.viewportHeight))
        val shown = list.filter { it.item.objectId !in scene.hidden }
        return if (visible == null) shown else EditorDrawing.touching(scene, shown, visible)
    }

    /** The saved objects of [layers] (all, when null) that can reach the screen, in drawing order, hidden ones left out. */
    private fun visibleOf(scene: CanvasScene, index: SavedIndex<DrawObject>, layers: Set<String>?): List<DrawObject> {
        val width = scene.camera.viewportWidth; val height = scene.camera.viewportHeight
        val visible = visibleDocumentRect(scene.matrix, Rect(0.0, 0.0, width, height))
        val candidates = if (visible == null) scene.objects else index.near(visible, EditorDrawing.cullMargin(scene)) ?: scene.objects
        val chosen = if (layers == null) candidates else candidates.filter { it.item.layerId in layers }
        val shown = chosen.filter { it.item.objectId !in scene.hidden }
        return if (visible == null) shown else EditorDrawing.touching(scene, shown, visible)
    }

    /** For each box of [boxes], the saved objects of [scene] (of [layers] only, when given) that can reach it, in drawing order. */
    private fun parts(scene: CanvasScene, index: SavedIndex<DrawObject>, boxes: List<PixelBox>, layers: Set<String>?): List<List<DrawObject>> {
        val moving = movingObjects(scene, index)
        val margin = EditorDrawing.cullMargin(scene)
        return boxes.map { box ->
            val visible = visibleDocumentRect(scene.matrix, Rect(box.left.toDouble(), box.top.toDouble(), (box.right - box.left).toDouble(), (box.bottom - box.top).toDouble()))
            val found = if (visible == null) scene.objects else EditorDrawing.touching(scene, index.near(visible, margin, moving) ?: scene.objects, visible)
            if (layers == null) found else found.filter { it.item.layerId in layers && it.item.objectId !in scene.hidden }
        }
    }

    /** The saved objects of [scene] that could reach the screen box [box], drawn ones only; null when that cannot be told. */
    private fun nearBox(scene: CanvasScene, index: SavedIndex<DrawObject>, box: PixelBox): List<DrawObject>? {
        val visible = visibleDocumentRect(scene.matrix, Rect(box.left.toDouble(), box.top.toDouble(), (box.right - box.left).toDouble(), (box.bottom - box.top).toDouble()))
            ?: return null
        return index.near(visible, EditorDrawing.cullMargin(scene), movingObjects(scene, index))?.filter { it.item.objectId !in scene.hidden }
    }

    /** The saved objects drawn elsewhere than they lie, for a gesture: wherever they are, the index finds them where they lie. */
    private fun movingObjects(scene: CanvasScene, index: SavedIndex<DrawObject>): List<DrawObject> =
        if (scene.transformed.isEmpty()) emptyList() else scene.transformed.keys.mapNotNull(index::find)

    /** Brings the pictures to [next], a scene with no gesture in it, at once:
     * all drawn again at its view when it moved otherwise than by whole pixels
     * with the page on the screen whole, else each by what it needs. False:
     * they could not be drawn (the frame is drawn whole). */
    private fun bring(post: Handler, scene: CanvasScene, next: PictureBasis<DrawObject>, index: SavedIndex<DrawObject>): Boolean {
        val now = held
        if (now == null || now.basis.width != next.width || now.basis.height != next.height) return renderAll(post, scene, next, index)
        val plan = planFor(now, scene, next, index) ?: return false
        if (softView(plan.move)) return renderAll(post, scene, next, index)
        val shift = plan.move as? ViewMove.Shift
        val dx = shift?.dx ?: 0; val dy = shift?.dy ?: 0
        val keep = HashMap<SlotKey, Slot>()
        for (slot in now.slots) keep[slot.key] = slot
        val needed = plan.layout.count { (plan.work[it] ?: SlotWork.Keep) != SlotWork.Keep || dx != 0 || dy != 0 || keep[it] == null }
        // Only composited otherwise (a layer hidden, made fainter, put elsewhere): no renderer is needed.
        val target = if (needed == 0) null else slotSurface(next.width, next.height) ?: return failed() != null
        if (target != null && !target.room(needed)) { roomShort = true; post.postDelayed(rest, REST_MS); return false }
        val made = ArrayList<Slot>(plan.layout.size)
        var rendered = 0; var appended = false; var patched = false; var moved = false
        try {
            for (key in plan.layout) {
                val work = plan.work[key] ?: SlotWork.Keep
                val before = keep[key]
                val layers = key.layers.toHashSet()
                val picture: PictureSurface.Picture? = when {
                    before == null || work == SlotWork.Render -> { rendered++; renderSlot(checkNotNull(target), scene, index, key, plan.paints) }
                    work == SlotWork.Keep && dx == 0 && dy == 0 -> before.picture
                    else -> {
                        val boxes = if (work == SlotWork.Patch) plan.area?.boxes.orEmpty() else emptyList()
                        val parts = if (boxes.isEmpty()) emptyList() else parts(scene, index, boxes, layers)
                        val added = ArrayList<DrawObject>()
                        if (work == SlotWork.Append) {
                            val step = plan.steps[key.layers.single()]
                            if (step is LayerStep.Append) { val list = plan.lists.of(key.layers.single()); for (at in step.from until list.size) added.add(list[at]) }
                            appended = true
                        }
                        if (work == SlotWork.Patch) patched = true
                        if (work == SlotWork.Keep) moved = true
                        val from = before.picture.bitmap
                        render(checkNotNull(target)) { EditorDrawing.drawSlotChanged(it, scene, from, dx, dy, key.prefix, boxes, parts, added, plan.paints) }
                    }
                }
                made.add(Slot(key, picture ?: throw RenderFailed()))
            }
        } catch (error: RenderFailed) {
            for (slot in made) if (keep[slot.key]?.picture !== slot.picture) slot.picture.release()
            return failed() != null
        }
        adopt(post, made, next, plan.lists)
        if (rendered == plan.layout.size && plan.layout.isNotEmpty()) counts.rebuilt.incrementAndGet() else if (rendered > 0) counts.layers.addAndGet(rendered.toLong())
        if (appended) counts.added.incrementAndGet()
        if (patched) { counts.patched.incrementAndGet(); plan.area?.let { counts.area.set(it.pixels) } }
        if (moved || (dx != 0 || dy != 0)) counts.shifted.incrementAndGet()
        return true
    }

    private class RenderFailed : RuntimeException()

    /** Every picture of [next] drawn from its objects at its view. */
    private fun renderAll(post: Handler, scene: CanvasScene, next: PictureBasis<DrawObject>, index: SavedIndex<DrawObject>): Boolean {
        val target = slotSurface(next.width, next.height) ?: return failed() != null
        val lists = listsOf(next.objects, next.layerIds)
        val drawn = lists.order.filter { firstDrawn(lists.of(it), next.hidden) { one -> one.item } != null }
        val layout = slotLayout(drawn, MAX_SLOTS)
        val paints = HashMap<String, RenderItem>()
        for (layer in lists.order) firstDrawn(lists.of(layer), next.hidden) { it.item }?.let { paints[layer] = it.item }
        if (!target.room(layout.size)) { roomShort = true; post.postDelayed(rest, REST_MS); return false }
        val made = ArrayList<Slot>(layout.size)
        for (key in layout) {
            val picture = renderSlot(target, scene, index, key, paints)
            if (picture == null) { made.forEach { it.picture.release() }; return failed() != null }
            made.add(Slot(key, picture))
        }
        adopt(post, made, next, lists)
        counts.rebuilt.incrementAndGet()
        return true
    }

    private fun renderSlot(target: PictureSurface, scene: CanvasScene, index: SavedIndex<DrawObject>, key: SlotKey, paints: Map<String, RenderItem>): PictureSurface.Picture? {
        // A small layer is looked through itself, not through everything the page shows.
        val own = if (key.prefix) null else listsOf(scene.objects, layerIds).of(key.layers.single()).takeIf { it.size <= SMALL_LAYER }
        val objects = if (own != null) visibleOfList(scene, own) else visibleOf(scene, index, key.layers.toHashSet())
        return render(target) { EditorDrawing.drawSlot(it, scene, key.prefix, objects, paints) }
    }

    private fun render(target: PictureSurface, draw: (Canvas) -> Unit): PictureSurface.Picture? {
        val retriedBefore = target.retried
        val made = target.render(draw)
        counts.retried.addAndGet((target.retried - retriedBefore).toLong())
        return made
    }

    /** The surface pictures of [width] x [height] are drawn with, made when there is none. */
    private fun slotSurface(width: Int, height: Int): PictureSurface? {
        handler?.removeCallbacks(rest)
        surface?.takeIf { !it.closed && it.width == width && it.height == height }?.let { return it }
        return try { surface?.close(); PictureSurface(width, height, SLOT_PICTURES).also { surface = it } }
        catch (error: RuntimeException) { surface = null; null }
    }

    /** [made] become the pictures held, for [next]. A picture no longer held
     * may still be in a frame waiting to be handed to the renderer; every such
     * frame is handed over by a message already in this thread's queue, so a
     * message added now runs after all of them: only then is it given back. */
    private fun adopt(post: Handler, made: List<Slot>, next: PictureBasis<DrawObject>, lists: LayerLists<DrawObject>) {
        val before = held
        held = Held(made, next, lists)
        failures = 0
        planHeld = null; planScene = null; planAnswer = null
        val keptPictures = made.mapTo(HashSet()) { it.picture }
        val gone = before?.slots?.map { it.picture }?.filter { it !in keptPictures }.orEmpty()
        if (gone.isNotEmpty()) { replaced.addAll(gone); post.post { synchronized(this) { for (old in gone) { replaced.remove(old); old.release() } } } }
        post.postDelayed(rest, REST_MS)
    }

    private fun failed(): PictureSurface.Picture? {
        failures++; counts.failed.incrementAndGet(); counts.whole.incrementAndGet()
        forget()
        return null
    }

    /** The overview brought to [scene] when the canvas is idle: drawn in part
     * where a few objects changed, else made again from every object. */
    private fun updateOverview(post: Handler, scene: CanvasScene) {
        val pageWidth = scene.document.width.toDouble(); val pageHeight = scene.document.height.toDouble()
        val width = scene.camera.viewportWidth.toInt(); val height = scene.camera.viewportHeight.toInt()
        val scale = overviewScale(pageWidth, pageHeight, width, height) ?: return
        val ow = maxOf(1, kotlin.math.ceil(pageWidth * scale).toInt()); val oh = maxOf(1, kotlin.math.ceil(pageHeight * scale).toInt())
        val lists = listsOf(scene.objects, layerIds)
        val paints = HashMap<String, RenderItem>()
        for (layer in lists.order) firstDrawn(lists.of(layer), scene.hidden) { it.item }?.let { paints[layer] = it.item }
        val keys = paintKeys(lists, paints)
        val kept = overview
        if (kept != null && kept.objects === scene.objects && (kept.hidden === scene.hidden || kept.hidden == scene.hidden) && kept.background === scene.background &&
            kept.smooth == scene.smoothBackground && kept.layerIds == layerIds && kept.paints == keys && kept.scale == scale) return
        val target = overviewSurface?.takeIf { !it.closed && it.width == ow && it.height == oh }
            ?: try { overviewSurface?.close(); PictureSurface(ow, oh, OVERVIEW_PICTURES).also { overviewSurface = it } } catch (error: RuntimeException) { overviewSurface = null; return }
        if (!target.room(1)) return
        val laidOut = scene.copy(matrix = Transform(a = scale, d = scale), camera = scene.camera.copy(scale = scale, viewportWidth = ow.toDouble(), viewportHeight = oh.toDouble()),
            transformed = emptyMap(), provisional = emptyList())
        val index = indexFor(scene)
        // In part: the same layers composited alike, the same source, and few objects changed, all of them placed.
        var picture: PictureSurface.Picture? = null
        var patched = false
        if (kept != null && kept.scale == scale && kept.background === scene.background && kept.smooth == scene.smoothBackground && kept.layerIds == layerIds &&
            kept.paints == keys && (kept.hidden === scene.hidden || kept.hidden == scene.hidden)) {
            val diff = objectsDiff(kept.objects, scene.objects, MAX_CHANGED)
            if (diff != null) {
                val area = TouchedArea(ow, oh)
                var fits = true
                for (at in diff.removed) if (!addDrawn(area, kept.objects[at].item, laidOut.matrix, null, OVERVIEW_MARGIN_PX)) { fits = false; break }
                if (fits) for (at in diff.added) if (!addDrawn(area, scene.objects[at].item, laidOut.matrix, null, OVERVIEW_MARGIN_PX)) { fits = false; break }
                if (fits && !area.large) {
                    val boxes = area.boxes.toList()
                    val parts = boxes.map { box ->
                        val visible = visibleDocumentRect(laidOut.matrix, Rect(box.left.toDouble(), box.top.toDouble(), (box.right - box.left).toDouble(), (box.bottom - box.top).toDouble()))
                        val found = if (visible == null) scene.objects else EditorDrawing.touching(laidOut, index.near(visible, EditorDrawing.cullMargin(laidOut)) ?: scene.objects, visible)
                        found.filter { it.item.objectId !in scene.hidden }
                    }
                    val from = kept.picture.bitmap
                    picture = render(target) { EditorDrawing.drawOverviewPatched(it, laidOut, from, boxes, parts, paints) } ?: return
                    patched = true
                }
            }
        }
        if (picture == null) {
            val all = scene.objects.filter { it.item.objectId !in scene.hidden }
            picture = render(target) { EditorDrawing.drawOverview(it, laidOut, all, paints) } ?: return
        }
        overview = Overview(picture, scale, scene.objects, scene.hidden, layerIds, keys, scene.background, scene.smoothBackground, pageWidth, pageHeight)
        if (patched) counts.overviewPatches.incrementAndGet() else counts.overviews.incrementAndGet()
        kept?.picture?.let { old -> replaced.add(old); post.post { synchronized(this) { replaced.remove(old); old.release() } } }
        post.removeCallbacks(rest); post.postDelayed(rest, REST_MS)
    }

    /** The saved objects of [scene] by place and id: the index held, brought
     * along by what changed, or a new one when too much did. */
    private fun indexFor(scene: CanvasScene): SavedIndex<DrawObject> {
        val objects = scene.objects
        val width = scene.document.width.toDouble(); val height = scene.document.height.toDouble()
        val last = index
        if (last != null && last.pageWidth == width && last.pageHeight == height) {
            if (last.list === objects) return last
            diffOf(last.list, objects)?.let { last.follow(objects, it); return last }
        }
        return SavedIndex<DrawObject>(width, height, { it.item.bounds }, { EditorDrawing.cullable(it.item, it.marker != null) }, { it.item.objectId })
            .also { it.reset(objects); index = it }
    }

    private fun refused(reason: PictureRefusal): Boolean {
        if (!reason.keepsPicture) forget()
        counts.whole.incrementAndGet(); counts.refused.incrementAndGet(reason.ordinal); counts.soft.set(false)
        return false
    }

    /** On-device tests only, right after [draw] returned true for [scene]: the
     * frame as it was just drawn, and the same scene drawn whole. */
    @Synchronized fun compare(scene: CanvasScene): PictureComparison? {
        val now = held ?: return null
        val plan = lastPlan ?: return null
        val pictures = try { PictureSurface(now.basis.width, now.basis.height) } catch (error: RuntimeException) { return null }
        // Both are drawn before either is read: a picture is read back only
        // once the surface has drawn everything it is asked for.
        var kept: PictureSurface.Picture? = null
        var whole: PictureSurface.Picture? = null
        try {
            kept = pictures.render {
                EditorDrawing.drawFrame(it, scene, plan)
                PrecisionDrawing.drawPanes(it, scene, onlyVisible = true)
            } ?: return null
            whole = pictures.render { PrecisionDrawing.draw(it, scene) } ?: return null
            val keptPixels = kept.bitmap.copy(Bitmap.Config.ARGB_8888, false) ?: return null
            val wholePixels = whole.bitmap.copy(Bitmap.Config.ARGB_8888, false) ?: run { keptPixels.recycle(); return null }
            return PictureComparison(keptPixels, wholePixels, scene.matrix, scene.document.width.toDouble(), scene.document.height.toDouble())
        } finally { kept?.release(); whole?.release(); pictures.close() }
    }

    /** Any thread: the pictures are given up as soon as the drawing thread
     * can, and made again by the next frame that wants them. */
    fun requestDrop() {
        dropAsked = true
        handler?.post(drop)
    }

    /** Nothing is held any more. Frames already drawn with a picture keep
     * it until they have been shown: giving it up never changes its pixels. */
    @Synchronized fun forget() {
        handler?.let { it.removeCallbacks(rest); it.removeCallbacks(settle); it.removeCallbacks(idle) }
        wanted = null; wantedScene = null; lastPlan = null; restingScene = null
        surface?.close(); surface = null
        overviewSurface?.close(); overviewSurface = null
        held?.slots?.forEach { it.picture.release() }; held = null
        overview?.picture?.release(); overview = null
        replaced.forEach { it.release() }; replaced.clear()
        planHeld = null; planScene = null; planAnswer = null
        listsFrom = null; listsIds = null; listsAnswer = null
        layerIdsOf = null; layerIds = emptyList(); index = null; diffFrom = null; diffTo = null; diffAnswer = null
        lastMatrix = null; lastMoving = false; counts.soft.set(false)
    }

    /** Main thread, once the session has been released: everything is given
     * up, and nothing is made from here on. */
    @Synchronized fun discard() {
        discarded = true; dropAsked = false
        forget()
        handler?.removeCallbacksAndMessages(null); handler = null
    }

    private companion object {
        /** After this long without a change the renderers and their spare buffers go. */
        const val REST_MS = 1_500L
        /** After this long without a frame, pictures a movement left unmade are made. */
        const val PICTURE_SETTLE_MS = 120L
        /** After this long without a frame, the overview is brought up to date. */
        const val OVERVIEW_IDLE_MS = 400L
        /** A session whose pictures keep failing to draw stops trying. */
        const val MAX_FAILURES = 3
        /** More changed objects in a layer than this are not followed one by one: its picture is drawn again. */
        const val MAX_CHANGED = 1_024
        /** Layers kept in pictures of their own; beyond that the lower ones share one. */
        const val MAX_SLOTS = 4
        /** Layer pictures held at once: the pictures shown, the ones being drawn, and those waiting to be given back. */
        const val SLOT_PICTURES = 3 * MAX_SLOTS
        const val OVERVIEW_PICTURES = 3
        /** Pictures drawn softer reach this much further on the screen; the overview's edges are this much softer. */
        const val SOFT_MARGIN_PX = 3.0
        const val OVERVIEW_MARGIN_PX = 2.0
        /** A layer of at most this many objects is drawn from its own list, not through the index. */
        const val SMALL_LAYER = 512
    }
}
