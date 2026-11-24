package com.visualworkbench.android.editor

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import com.visualworkbench.android.precision.PrecisionDrawing
import com.visualworkbench.shared.DocumentSnapshot
import com.visualworkbench.shared.Rect
import com.visualworkbench.shared.RenderItem
import com.visualworkbench.shared.Shape
import com.visualworkbench.shared.Transform
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicLongArray

// The canvas shows a kept picture of the saved page, as a paint program does.
// The picture is made once for a view, each finished stroke is drawn into it
// once, a change to the page (undo, redo, an erasure, a move, a turn, a
// deletion, a restyle, a change from the paired computer) draws again only the
// part of the picture it touched, and a frame is the picture with the stroke in
// progress and the overlays over it. While a stroke is being moved or turned,
// each frame draws the part the gesture changes over the picture. The rules
// for when the picture stands, when a stroke is added to it, when a part of it
// is drawn again and when it is made again are plain functions here and in
// TouchedArea.kt, tested without a phone; the drawing is at the end of the file.

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

/** What has to be known about the saved objects of a scene to put something
 * on top of them, or to draw a part of them again: the order the layers are
 * drawn in, how many objects each layer draws, which layers have anything
 * drawn, and how each layer is composited. Worked out once for a list of saved
 * objects, and brought along, not worked out again, when objects are added
 * ([add]) or taken away ([remove]).
 *
 * [layers] are the document's layer ids in order, [saved] the saved objects in
 * drawing order; [hidden] objects are not drawn. */
internal class DryLayers private constructor(
    // How many layers the document lists: they come first in [place].
    private val listed: Int,
    // Layer order as the renderer builds it: the document's layers, then any other in first-use order.
    private val place: LinkedHashMap<String, Int>,
    // The whole layer is composited with the opacity and blend of its first drawn object.
    private val first: HashMap<String, RenderItem>,
    // How many objects each layer draws.
    private val drawn: HashMap<String, Int>,
    private var topDrawn: Int,
    // Some layer draws objects that are not all composited alike.
    private var mixed: Boolean,
) {
    constructor(layers: List<String>, saved: List<RenderItem>, hidden: Set<String>) :
        this(LinkedHashMap<String, Int>().also { place -> for (id in layers) place.getOrPut(id) { place.size } }, saved, hidden)
    private constructor(place: LinkedHashMap<String, Int>, saved: List<RenderItem>, hidden: Set<String>) :
        this(place.size, place, HashMap(), HashMap(), -1, false) { add(saved, hidden) }

    /** [saved] follow, in drawing order, everything this was told of before. */
    fun add(saved: List<RenderItem>, hidden: Set<String>) {
        for (item in saved) {
            if (item.objectId in hidden) continue
            val at = place.getOrPut(item.layerId) { place.size }
            if (at > topDrawn) topDrawn = at
            drawn[item.layerId] = (drawn[item.layerId] ?: 0) + 1
            val paint = first[item.layerId]
            if (paint == null) first[item.layerId] = item else if (!samePaint(paint, item)) mixed = true
        }
    }

    /** [saved], each drawn before, are drawn no longer. False: what is left
     * cannot be told without starting over (a layer whose objects are not all
     * composited alike, whose first may now be another; or a layer the
     * document does not list, left empty), and this is left half done. */
    fun remove(saved: List<RenderItem>, hidden: Set<String>): Boolean {
        if (mixed) return false
        var emptied = false
        for (item in saved) {
            if (item.objectId in hidden) continue
            val count = drawn[item.layerId] ?: return false
            if (count > 1) { drawn[item.layerId] = count - 1; continue }
            drawn.remove(item.layerId); first.remove(item.layerId); emptied = true
            if ((place[item.layerId] ?: return false) >= listed) return false
        }
        if (emptied) {
            topDrawn = -1
            for (layer in drawn.keys) { val at = place.getValue(layer); if (at > topDrawn) topDrawn = at }
        }
        return true
    }

    fun copy(): DryLayers = DryLayers(listed, LinkedHashMap(place), HashMap(first), HashMap(drawn), topDrawn, mixed)

    /** Whether the stack can be drawn by parts: every layer draws objects
     * composited alike, and every layer drawn is one the document lists, so
     * that the layers come in the document's order whatever is drawn. */
    val clean: Boolean get() = !mixed && place.size == listed

    /** The first drawn object of [layer], which says how the layer is composited; null when it draws nothing. */
    fun paintOf(layer: String): RenderItem? = first[layer]

    /** Whether the document lists [layer]. */
    fun listed(layer: String): Boolean = (place[layer] ?: Int.MAX_VALUE) < listed

    /** Whether [layer] has a place in the order (the document lists it, or something drawn is in it). */
    fun placed(layer: String): Boolean = place.containsKey(layer)

    /** Whether [wet] can be painted straight over a finished picture of the
     * saved objects and look exactly as when everything is drawn together.
     * That is so only when all of it sits in the layer drawn last and that
     * layer is composited plainly: full opacity, normal blend. A highlighter
     * (multiply) or a stroke beneath another layer needs the whole scene, and
     * gets it. The same holds for saved objects added on top of a picture. */
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
     * erased, and what that changes cannot be drawn over a picture: none is
     * held, or the change covers too much of the screen or cannot be placed. */
    Gesture(true),
}

/** Null when a frame may be drawn from a kept picture. [width] and [height]
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

/** Everything the pixels of a kept picture depend on. The picture holds the
 * saved document as it is stacked: the source pixels and every object layer,
 * transparent elsewhere. The surround, the white page, the page's edge, ink in
 * progress, selection, hover and the magnifier are drawn over or under it for
 * each frame, so the theme and those are not part of this. A picture is only
 * ever made from a scene with no gesture in it: no object drawn elsewhere
 * than it lies, and none hidden only for an erasure in progress.
 *
 * [objects] and [background] are compared by identity: a saved object is
 * never changed in place, and an edit, an undo, an erasure, a move, a change
 * from the paired computer and another document or page all arrive as other
 * objects. */
internal class PictureBasis<T>(val width: Int, val height: Int, val matrix: Transform, val scale: Double,
    val background: Any?, val smoothBackground: Boolean, val layerIds: List<String>, val hidden: Set<String>, val objects: List<T>)

/** What made a kept picture wrong as a whole, so that it is made again. */
internal enum class PictureChange {
    Missing, Size, View, Background, Layers,
    /** The saved objects changed in a way that cannot be drawn by parts: an
     * object whose drawing its box does not hold, a layer whose objects are not
     * all composited alike or that the document does not list, a change of a
     * layer's compositing, or more objects than are followed one by one. */
    Objects,
    /** What changed covers more than half of the screen. */
    Area,
}

internal sealed interface PictureStep {
    /** The picture is the saved scene. */
    data object Keep : PictureStep
    /** The picture is the saved scene without its objects from [from] on, and
     * those are drawn after everything else in a plainly composited layer:
     * they are drawn into the picture, and nothing else is. */
    data class Append(val from: Int) : PictureStep
    /** The picture is the saved scene outside [area]: that part is drawn
     * again, and nothing else is. An empty area changes no pixel. */
    data class Patch(val area: TouchedArea) : PictureStep
    /** The picture is made again from every object. */
    data class Rebuild(val why: PictureChange) : PictureStep
}

/** What, besides the saved objects, makes a picture drawn from [held] wrong for [next]; null when nothing does. */
internal fun <T> frameChange(held: PictureBasis<T>, next: PictureBasis<T>): PictureChange? = when {
    held.width != next.width || held.height != next.height -> PictureChange.Size
    held.matrix != next.matrix || held.scale != next.scale -> PictureChange.View
    held.background !== next.background || held.smoothBackground != next.smoothBackground -> PictureChange.Background
    held.layerIds != next.layerIds -> PictureChange.Layers
    else -> null
}

/** What to do with a picture drawn from [held] so that it shows [next], a
 * scene with no gesture in it. [layers] is what [DryLayers] holds for
 * [held]'s objects, [diff] tells two lists of objects apart ([objectsDiff],
 * null when more changed than is followed), and [find] finds an object of
 * [next] by its id. */
internal fun <T : Any> pictureStep(held: PictureBasis<T>?, layers: DryLayers?, next: PictureBasis<T>,
    diff: (List<T>, List<T>) -> ObjectsDiff?, find: (String) -> T?, item: (T) -> RenderItem): PictureStep {
    if (held == null || layers == null) return PictureStep.Rebuild(PictureChange.Missing)
    frameChange(held, next)?.let { return PictureStep.Rebuild(it) }
    val sameHidden = held.hidden === next.hidden || held.hidden == next.hidden
    val changed = diff(held.objects, next.objects) ?: return PictureStep.Rebuild(PictureChange.Objects)
    if (changed.isEmpty && sameHidden) return PictureStep.Keep
    if (sameHidden && changed.appended(held.objects.size, next.objects.size)) {
        val added = ArrayList<RenderItem>(changed.added.size)
        for (at in changed.added) added.add(item(next.objects[at]))
        if (layers.inkOnTop(added, next.hidden)) return PictureStep.Append(held.objects.size)
    }
    val area = touchedArea(held, layers, next, emptyMap(), changed, find, item) ?: return PictureStep.Rebuild(PictureChange.Objects)
    return if (area.large) PictureStep.Rebuild(PictureChange.Area) else PictureStep.Patch(area)
}

/** Where on the screen the saved scene [next] differs from a picture drawn
 * from [held] (whose objects [layers] describes; [diff] tells their lists
 * apart), when [next]'s objects are drawn with [transformed] in place of their
 * own transforms, as a gesture shows them: where the picture shows an object
 * the frame does not, where the frame shows one the picture does not, where
 * an object being moved or turned was and is, and where an object hidden or
 * shown again lies. [find] finds an object of [next] by its id.
 *
 * Null when the difference cannot be drawn by parts: an object whose drawing
 * its box does not hold ([boxHoldsDrawing]), a picture whose layers are not
 * [DryLayers.clean], an object shown in a layer the document does not list,
 * or composited otherwise than the objects its layer already draws. */
internal fun <T : Any> touchedArea(held: PictureBasis<T>, layers: DryLayers, next: PictureBasis<T>, transformed: Map<String, Transform>,
    diff: ObjectsDiff, find: (String) -> T?, item: (T) -> RenderItem): TouchedArea? {
    if (!layers.clean) return null
    val area = TouchedArea(next.width, next.height)
    val view = next.matrix
    // A layer the picture draws nothing in takes the compositing of the first object it shows now.
    var fresh: HashMap<String, RenderItem>? = null
    fun fits(shown: RenderItem): Boolean {
        if (!layers.listed(shown.layerId)) return false
        val paint = layers.paintOf(shown.layerId) ?: (fresh ?: HashMap<String, RenderItem>().also { fresh = it }).getOrPut(shown.layerId) { shown }
        return samePaint(paint, shown)
    }
    for (at in diff.removed) if (!addDrawn(area, item(held.objects[at]), view, null)) return null
    for (at in diff.added) {
        val shown = item(next.objects[at])
        if (!addDrawn(area, shown, view, transformed[shown.objectId])) return null
        if (shown.objectId !in next.hidden && !fits(shown)) return null
    }
    for ((id, preview) in transformed) {
        val moved = find(id)?.let(item) ?: continue
        if (preview == moved.transform) continue
        if (!addDrawn(area, moved, view, null) || !addDrawn(area, moved, view, preview)) return null
    }
    if (held.hidden !== next.hidden && held.hidden != next.hidden) {
        for (id in next.hidden) if (id !in held.hidden) {
            val gone = find(id)?.let(item) ?: continue
            if (!addDrawn(area, gone, view, null) || !addDrawn(area, gone, view, transformed[id])) return null
        }
        for (id in held.hidden) if (id !in next.hidden) {
            val back = find(id)?.let(item) ?: continue
            if (!addDrawn(area, back, view, null) || !addDrawn(area, back, view, transformed[id]) || !fits(back)) return null
        }
    }
    return area
}

/** Whether [a] and [b] are the same saved scene, as far as a picture goes. */
internal fun <T : Any> sameBasis(a: PictureBasis<T>, b: PictureBasis<T>, diff: (List<T>, List<T>) -> ObjectsDiff?): Boolean =
    frameChange(a, b) == null && (a.hidden === b.hidden || a.hidden == b.hidden) && diff(a.objects, b.objects)?.isEmpty == true

/** Whether a picture that has to be made again ([why]) is made for this
 * frame, or one drawn again in part right after it. It is, unless the scene is
 * changing from frame to frame, as it does while the page is panned, zoomed or
 * turned and while the paired computer moves an object: then each frame would
 * make a picture that the next one throws away. Such a frame is drawn over the
 * picture held where only a part differs from it, and whole otherwise, and the
 * picture is made once the scene has stopped changing. A change of view or
 * size is taken to be the start of such a movement; any other change is made
 * at once unless it follows another closely. [sameAsLastFrame]: the frame
 * before wanted this very picture. [sinceChangeMs]: how long ago a frame last
 * wanted another picture than the one before it, negative when none ever did. */
internal fun pictureMadeNow(sameAsLastFrame: Boolean, sinceChangeMs: Long, why: PictureChange): Boolean =
    sameAsLastFrame || (why != PictureChange.View && why != PictureChange.Size && (sinceChangeMs < 0 || sinceChangeMs >= PICTURE_STREAK_MS))

/** Changes closer together than this are one movement. */
internal const val PICTURE_STREAK_MS = 100L

/** How often each thing happened, for the on-device tests and nothing else. */
internal class PictureCounts {
    /** Pictures made from every object, pictures a stroke was added to,
     * pictures a part of which was drawn again, frames drawn from a picture,
     * frames drawn from a picture with a part drawn again over it, frames
     * drawn as a whole scene, and renders that failed. */
    val rebuilt = AtomicLong(); val added = AtomicLong(); val patched = AtomicLong(); val frames = AtomicLong(); val over = AtomicLong()
    val whole = AtomicLong(); val failed = AtomicLong()
    /** Pictures that came out only once their surface was set again ([PictureSurface.render]). */
    val retried = AtomicLong()
    /** The pixels of the part last drawn again, into a picture or over it. */
    val area = AtomicLong()
    /** Frames drawn whole, by [PictureRefusal]. */
    val refused = AtomicLongArray(PictureRefusal.entries.size)
    override fun toString(): String = "rebuilt=${rebuilt.get()} added=${added.get()} patched=${patched.get()} frames=${frames.get()} over=${over.get()} " +
        "whole=${whole.get()} failed=${failed.get()} retried=${retried.get()} area=${area.get()}" +
        PictureRefusal.entries.joinToString("") { reason -> refused.get(reason.ordinal).let { if (it == 0L) "" else " ${reason.name.lowercase()}=$it" } }
}

/** On-device tests only: one frame as the kept picture draws it and the same
 * scene drawn whole, both as plain pixels, and where the page lies in them
 * ([matrix] maps the page of [pageWidth] x [pageHeight] to the frame). */
internal class PictureComparison(val kept: Bitmap, val whole: Bitmap, val matrix: Transform, val pageWidth: Double, val pageHeight: Double)

/** The kept picture of one render session, and the frames drawn from it.
 *
 * [draw] draws a whole frame when the scene allows it. For a scene with no
 * gesture in it, it brings the picture up to the saved scene first (nothing to
 * do, a stroke to add, or all of it again) and then draws the surround, the
 * page, the picture, the ink in progress, the overlays and the magnifier. A
 * change that touched a part of the page (undo, an erasure, a move, a
 * change from the paired computer) is drawn over the picture in that frame,
 * and into the picture by a message right behind it, so that no frame waits
 * for a picture to be drawn off screen. For a scene with a gesture in it
 * (objects being moved or turned, objects hidden while erased) it leaves the
 * picture as it is and draws, over it, the part of the screen the gesture
 * changes. When it returns false nothing was drawn and the caller draws the
 * whole scene as it always could.
 *
 * A part of the screen is drawn from the saved objects a [SavedIndex] finds
 * there, and the pen's loupe, which redraws the page near the nib at every
 * frame, finds them the same way: no part of a frame visits every saved
 * object. The index follows the saved objects as they change, by the objects
 * that changed ([objectsDiff]), and so does the layer order ([DryLayers]).
 *
 * While the scene changes from frame to frame (a pan, a zoom, a turn, an
 * object moved from the paired computer) no picture is made for each frame:
 * those frames are drawn over the picture held where only a part differs, and
 * whole otherwise, and the picture is made when the movement ends
 * ([pictureMadeNow]).
 *
 * Memory: one screen-sized picture while the page rests. While the picture
 * changes a second buffer is kept to draw the next state into, and at most
 * two more wait a moment to be released; [REST_MS] after the last change the
 * renderer and its spare buffers are given up and only the picture stays.
 * The index holds a few references for each saved object. Everything goes at
 * [forget] and [discard], and nothing is made after [discard].
 *
 * Owned by one render session. [draw] and [compare] run on its drawing
 * thread, and so does everything this posts to that thread. [requestDrop] may
 * be called from any thread. [discard] runs on the main thread once the
 * session has been released; a message of this class still waiting on the
 * drawing thread may run beside it, so every entry holds this object's lock,
 * which nothing else ever contends for. */
internal class KeptPicture(val counts: PictureCounts = PictureCounts()) {
    private class Held(val picture: PictureSurface.Picture, val basis: PictureBasis<DrawObject>, val layers: DryLayers)

    /** The layer order of the saved objects last seen, kept whether or not a
     * picture is: a highlighter stroke asks for it at every frame. */
    private class Judged(val objects: List<DrawObject>, val hidden: Set<String>, val layerIds: List<String>, val layers: DryLayers)

    /** What the last frame drew again over the picture, for [compare]. */
    private class Over(val boxes: List<PixelBox>, val parts: List<List<DrawObject>>)

    private var surface: PictureSurface? = null
    private var held: Held? = null
    private var judged: Judged? = null
    /** The saved objects of the scene last drawn, found by place and by id. */
    private var index: SavedIndex<DrawObject>? = null
    /** The last answer of [diffOf]: the picture, the index and the layer order
     * each ask it of the same two lists after a change. */
    private var diffFrom: List<DrawObject>? = null
    private var diffTo: List<DrawObject>? = null
    private var diffAnswer: ObjectsDiff? = null
    private fun diffOf(from: List<DrawObject>, to: List<DrawObject>): ObjectsDiff? {
        if (from === to) return ObjectsDiff.NONE
        if (from !== diffFrom || to !== diffTo) { diffAnswer = objectsDiff(from, to, MAX_CHANGED); diffFrom = from; diffTo = to }
        return diffAnswer
    }
    private var layerIdsOf: DocumentSnapshot? = null
    private var layerIds: List<String> = emptyList()
    private var failures = 0
    private var discarded = false
    private var lastOver: Over? = null
    /** Pictures replaced and not yet released. */
    private val replaced = ArrayList<PictureSurface.Picture>(2)
    /** The picture the last frame wanted and did not get, the scene it is
     * drawn from, and when a frame last wanted another picture than the one before it. */
    private var wanted: PictureBasis<DrawObject>? = null
    private var wantedScene: CanvasScene? = null
    private var changedMs = 0L
    @Volatile private var handler: Handler? = null
    @Volatile private var dropAsked = false
    private val rest = Runnable { synchronized(this) { surface?.close(); surface = null } }
    private val drop = Runnable { synchronized(this) { if (dropAsked) { dropAsked = false; forget() } } }
    /** Right after a frame drawn over the picture, or once a movement has
     * ended and no frame has come since: the picture is brought up to date
     * for the next one. */
    private val settle = Runnable {
        synchronized(this) {
            val basis = wanted; val scene = wantedScene; val post = handler
            wanted = null; wantedScene = null
            if (!discarded && basis != null && scene != null && post != null && failures < MAX_FAILURES) {
                val index = indexFor(scene)
                update(post, scene, basis, pictureStep(held?.basis, held?.layers, basis, ::diffOf, index::find) { it.item }, index)
            }
        }
    }

    /** [steady]: no saved object is hidden for an erasure in progress.
     * [resizing]: the session owes the redraw of an in-place resize.
     * [first]: the session has not shown a frame yet. Its first frame is drawn
     * whole, so that a new canvas appears no later than it used to; the
     * picture is made right after, before a pen can need it. */
    @Synchronized fun draw(canvas: Canvas, scene: CanvasScene, steady: Boolean, resizing: Boolean, first: Boolean): Boolean {
        lastOver = null
        if (discarded) return false
        if (dropAsked) { dropAsked = false; forget() }
        val width = scene.camera.viewportWidth.toInt(); val height = scene.camera.viewportHeight.toInt()
        val refusal = pictureRefusal(canvas.isHardwareAccelerated, canvas.width, canvas.height, width, height, resizing,
            giant = scene.giantMode || scene.giantFrame != null, documentTiles = scene.documentFrame != null,
            resultsReady = scene.resultDisplayReady, results = scene.aiResults.isNotEmpty(), draft = scene.documentDraft != null)
        if (refusal != null) return refused(refusal)
        // Releasing a replaced picture is posted to this thread, behind every frame drawn before.
        val looper = Looper.myLooper()
        if (looper == null || failures >= MAX_FAILURES) { counts.whole.incrementAndGet(); return false }
        val post = handler?.takeIf { it.looper === looper } ?: Handler(looper).also { handler = it }
        val ids = if (layerIdsOf === scene.document) layerIds else scene.document.layers.map { it.id }.also { layerIds = it; layerIdsOf = scene.document }
        if (scene.provisional.isNotEmpty() && !judge(scene, ids).inkOnTop(scene.provisional.map { it.item }, scene.hidden)) {
            counts.whole.incrementAndGet(); return false
        }
        val index = indexFor(scene)
        val next = PictureBasis(width, height, scene.matrix, scene.camera.scale, scene.background, scene.smoothBackground, ids, scene.hidden, scene.objects)
        val now = held
        if (!steady || scene.transformed.isNotEmpty()) {
            // A gesture is never made the picture: what it changes is drawn over the picture held.
            val area = now?.takeIf { frameChange(it.basis, next) == null }?.let { at ->
                diffOf(at.basis.objects, next.objects)?.let { touchedArea(at.basis, at.layers, next, scene.transformed, it, index::find) { drawn -> drawn.item } }
            }
            if (now == null || area == null || area.large) return refused(PictureRefusal.Gesture)
            return drawFrame(canvas, scene, now.picture, area, index)
        }
        val step = pictureStep(now?.basis, now?.layers, next, ::diffOf, index::find) { it.item }
        if (step is PictureStep.Patch && !step.area.isEmpty && now != null) {
            // The part that changed is drawn over the picture held, in this
            // frame; the picture itself is brought up to date by a message
            // right behind this frame on this thread, or, while the scene
            // changes from frame to frame, once it has stopped. No frame waits
            // for a picture to be drawn off screen.
            val uptime = SystemClock.uptimeMillis()
            val same = wanted?.let { sameBasis(it, next, ::diffOf) } == true
            val soon = pictureMadeNow(same, if (changedMs == 0L) -1 else uptime - changedMs, PictureChange.Objects)
            if (!same) changedMs = uptime
            wanted = next; wantedScene = scene
            post.removeCallbacks(settle)
            if (soon) post.post(settle) else post.postDelayed(settle, PICTURE_SETTLE_MS)
            return drawFrame(canvas, scene, now.picture, step.area, index)
        }
        if (step is PictureStep.Rebuild) {
            val uptime = SystemClock.uptimeMillis()
            val same = wanted?.let { sameBasis(it, next, ::diffOf) } == true
            val made = !first && pictureMadeNow(same, if (changedMs == 0L) -1 else uptime - changedMs, step.why)
            if (!same) changedMs = uptime
            if (!made) {
                // Made when the next frame wants the same, or when none comes; till then drawn whole.
                wanted = next; wantedScene = scene
                post.removeCallbacks(settle); post.postDelayed(settle, PICTURE_SETTLE_MS)
                counts.whole.incrementAndGet(); return false
            }
        }
        post.removeCallbacks(settle); wanted = null; wantedScene = null
        val picture = update(post, scene, next, step, index) ?: return false
        return drawFrame(canvas, scene, picture, null, index)
    }

    /** A frame from [picture], with [area] drawn again over it when given and not empty. */
    private fun drawFrame(canvas: Canvas, scene: CanvasScene, picture: PictureSurface.Picture, area: TouchedArea?, index: SavedIndex<DrawObject>): Boolean {
        // The selected object's outline, found by its id: no pass over the saved objects.
        val selection = scene.selected?.let(index::find)
        if (area == null || area.isEmpty) EditorDrawing.drawKept(canvas, scene, picture.bitmap, selection)
        else {
            val parts = parts(scene, index, area)
            EditorDrawing.drawOver(canvas, scene, picture.bitmap, area.boxes, parts, selection)
            lastOver = Over(area.boxes, parts)
            counts.over.incrementAndGet(); counts.area.set(area.pixels)
        }
        // The pen's loupe shows the page near the nib while the pen is down
        // or hovers: only the saved objects the index finds there are looked at.
        val moving = moving(scene, index)
        PrecisionDrawing.drawPanes(canvas, scene, onlyVisible = true) { visible, margin -> index.near(visible, margin, moving) }
        counts.frames.incrementAndGet()
        return true
    }

    /** Brings the picture to [next] by [step]; the picture to draw the frame
     * from, or null when it could not be drawn (the frame is drawn whole). */
    private fun update(post: Handler, scene: CanvasScene, next: PictureBasis<DrawObject>, step: PictureStep, index: SavedIndex<DrawObject>): PictureSurface.Picture? {
        val now = held
        return when (step) {
            PictureStep.Keep -> checkNotNull(now).also {
                if (it.basis.objects !== next.objects || it.basis.hidden !== next.hidden) held = Held(it.picture, next, it.layers)
            }.picture
            is PictureStep.Append -> {
                val before = checkNotNull(now).picture.bitmap
                replace(post, next, judge(scene, next.layerIds), counts.added) { EditorDrawing.drawAdded(it, scene, before, scene.objects.subList(step.from, scene.objects.size)) }
            }
            is PictureStep.Patch -> {
                val kept = checkNotNull(now)
                if (step.area.isEmpty) { held = Held(kept.picture, next, judge(scene, next.layerIds).copy()); kept.picture }
                else {
                    val parts = parts(scene, index, step.area)
                    val before = kept.picture.bitmap
                    replace(post, next, judge(scene, next.layerIds), counts.patched) { EditorDrawing.drawPatched(it, scene, before, step.area.boxes, parts) }
                        ?.also { counts.area.set(step.area.pixels) }
                }
            }
            is PictureStep.Rebuild -> replace(post, next, judge(scene, next.layerIds), counts.rebuilt) { EditorDrawing.drawStack(it, scene) }
        }
    }

    /** For each box of [area], the saved objects of [scene] that can reach it, in drawing order. */
    private fun parts(scene: CanvasScene, index: SavedIndex<DrawObject>, area: TouchedArea): List<List<DrawObject>> {
        val moving = moving(scene, index)
        val margin = EditorDrawing.cullMargin(scene)
        return area.boxes.map { box ->
            val visible = visibleDocumentRect(scene.matrix, Rect(box.left.toDouble(), box.top.toDouble(), (box.right - box.left).toDouble(), (box.bottom - box.top).toDouble()))
            if (visible == null) scene.objects
            else EditorDrawing.touching(scene, index.near(visible, margin, moving) ?: scene.objects, visible)
        }
    }

    /** The saved objects drawn elsewhere than they lie, for a gesture: wherever they are, the index finds them where they lie. */
    private fun moving(scene: CanvasScene, index: SavedIndex<DrawObject>): List<DrawObject> =
        if (scene.transformed.isEmpty()) emptyList() else scene.transformed.keys.mapNotNull(index::find)

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

    /** The layer order for [scene]'s saved objects: the one held, the one held
     * brought along by what changed, or a new one. */
    private fun judge(scene: CanvasScene, ids: List<String>): DryLayers {
        val last = judged
        val objects = scene.objects
        if (last != null && last.layerIds == ids && (last.hidden === scene.hidden || last.hidden == scene.hidden)) {
            if (last.objects === objects) return last.layers
            val change = diffOf(last.objects, objects)
            if (change != null) {
                val added = ArrayList<RenderItem>(change.added.size)
                for (at in change.added) added.add(objects[at].item)
                // A layer nobody listed takes its place by first use: one met first in the middle of the list is worked out again.
                val appended = change.appended(last.objects.size, objects.size)
                if (appended || added.all { last.layers.placed(it.layerId) }) {
                    val removed = ArrayList<RenderItem>(change.removed.size)
                    for (at in change.removed) removed.add(last.objects[at].item)
                    // A held picture has its own copy of what it was drawn from.
                    if (last.layers.remove(removed, scene.hidden)) {
                        last.layers.add(added, scene.hidden)
                        judged = Judged(objects, scene.hidden, ids, last.layers)
                        return last.layers
                    }
                }
            }
        }
        return DryLayers(ids, objects.map { it.item }, scene.hidden).also { judged = Judged(objects, scene.hidden, ids, it) }
    }

    private fun refused(reason: PictureRefusal): Boolean {
        if (!reason.keepsPicture) forget()
        counts.whole.incrementAndGet(); counts.refused.incrementAndGet(reason.ordinal)
        return false
    }

    /** Draws the next picture and makes it the one held. Null: it could not
     * be drawn (nothing is held any more), or there was no room for it (the
     * picture held stays as it was); the frame is drawn whole. */
    private fun replace(post: Handler, basis: PictureBasis<DrawObject>, layers: DryLayers, count: AtomicLong, draw: (Canvas) -> Unit): PictureSurface.Picture? {
        post.removeCallbacks(rest)
        val target = surface?.takeIf { !it.closed && it.width == basis.width && it.height == basis.height }
            ?: try { surface?.close(); PictureSurface(basis.width, basis.height).also { surface = it } }
            catch (error: RuntimeException) { surface = null; return failed() }
        // Pictures replaced a moment ago are still waiting for the frames that
        // show them. Without room for another, this frame is drawn whole.
        if (!target.room) { post.postDelayed(rest, REST_MS); counts.whole.incrementAndGet(); return null }
        val retriedBefore = target.retried
        val made = target.render(draw)
        counts.retried.addAndGet((target.retried - retriedBefore).toLong())
        if (made == null) return failed()
        val old = held?.picture
        held = Held(made, basis, layers.copy())
        failures = 0; count.incrementAndGet()
        // A frame drawn before this one may still be waiting to be handed to
        // the renderer with the old picture in it. Every such frame is handed
        // over by a message already in this thread's queue, so a message added
        // now runs after all of them: only then may the old buffer be drawn
        // into again.
        if (old != null) { replaced.add(old); post.post { synchronized(this) { replaced.remove(old); old.release() } } }
        post.postDelayed(rest, REST_MS)
        return made
    }

    private fun failed(): PictureSurface.Picture? {
        failures++; counts.failed.incrementAndGet(); counts.whole.incrementAndGet()
        forget()
        return null
    }

    /** On-device tests only, right after [draw] returned true for [scene]: the
     * frame as it was just drawn, and the same scene drawn whole. */
    @Synchronized fun compare(scene: CanvasScene): PictureComparison? {
        val now = held ?: return null
        val over = lastOver
        val pictures = try { PictureSurface(now.basis.width, now.basis.height) } catch (error: RuntimeException) { return null }
        // Both are drawn before either is read: a picture is read back only
        // once the surface has drawn everything it is asked for.
        var kept: PictureSurface.Picture? = null
        var whole: PictureSurface.Picture? = null
        try {
            kept = pictures.render {
                if (over == null) EditorDrawing.drawKept(it, scene, now.picture.bitmap)
                else EditorDrawing.drawOver(it, scene, now.picture.bitmap, over.boxes, over.parts)
                PrecisionDrawing.drawPanes(it, scene, onlyVisible = true)
            } ?: return null
            whole = pictures.render { PrecisionDrawing.draw(it, scene) } ?: return null
            val keptPixels = kept.bitmap.copy(Bitmap.Config.ARGB_8888, false) ?: return null
            val wholePixels = whole.bitmap.copy(Bitmap.Config.ARGB_8888, false) ?: run { keptPixels.recycle(); return null }
            return PictureComparison(keptPixels, wholePixels, scene.matrix, scene.document.width.toDouble(), scene.document.height.toDouble())
        } finally { kept?.release(); whole?.release(); pictures.close() }
    }

    /** Any thread: the picture is given up as soon as the drawing thread can,
     * and made again by the next frame that wants it. */
    fun requestDrop() {
        dropAsked = true
        handler?.post(drop)
    }

    /** Nothing is held any more. Frames already drawn with the picture keep
     * it until they have been shown: giving it up never changes its pixels. */
    @Synchronized fun forget() {
        handler?.let { it.removeCallbacks(rest); it.removeCallbacks(settle) }
        wanted = null; wantedScene = null; lastOver = null
        surface?.close(); surface = null
        held?.picture?.release(); held = null
        replaced.forEach { it.release() }; replaced.clear()
        judged = null; layerIdsOf = null; layerIds = emptyList(); index = null; diffFrom = null; diffTo = null; diffAnswer = null
    }

    /** Main thread, once the session has been released: everything is given
     * up, and nothing is made from here on. */
    @Synchronized fun discard() {
        discarded = true; dropAsked = false
        forget()
        handler?.removeCallbacksAndMessages(null); handler = null
    }

    private companion object {
        /** After this long without a change the renderer and its spare buffers go. */
        const val REST_MS = 1_500L
        /** After this long without a frame, a picture a movement left unmade is made. */
        const val PICTURE_SETTLE_MS = 120L
        /** A session whose pictures keep failing to draw stops trying. */
        const val MAX_FAILURES = 3
        /** More changed objects than this are not followed one by one: the picture and the index are made again. */
        const val MAX_CHANGED = 1_024
    }
}
