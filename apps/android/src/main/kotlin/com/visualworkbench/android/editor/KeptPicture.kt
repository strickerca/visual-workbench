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

// The canvas shows a kept picture of the saved page, as a paint program does.
// The picture is made once for a view, each finished stroke is drawn into it
// once, and a frame is the picture with the stroke in progress and the
// overlays over it. The rules for when the picture stands, when a stroke is
// added to it and when it is made again are plain functions here, tested
// without a phone; the drawing is at the end of the file.

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

/** What has to be known about the saved objects of a scene to put something
 * on top of them: the order the layers are drawn in, the last layer that has
 * anything drawn, and how each layer is composited. Worked out once for a
 * list of saved objects and extended, not worked out again, when objects are
 * added at the end of that list ([add]).
 *
 * [layers] are the document's layer ids in order, [saved] the saved objects in
 * drawing order; [hidden] objects are not drawn. */
internal class DryLayers private constructor(
    // Layer order as the renderer builds it: the document's layers, then any other in first-use order.
    private val place: LinkedHashMap<String, Int>,
    // The whole layer is composited with the opacity and blend of its first drawn object.
    private val first: HashMap<String, RenderItem>,
    private var topDrawn: Int,
) {
    constructor(layers: List<String>, saved: List<RenderItem>, hidden: Set<String>) : this(LinkedHashMap(), HashMap(), -1) {
        for (id in layers) place.getOrPut(id) { place.size }
        add(saved, hidden)
    }

    /** [saved] follow, in drawing order, everything this was told of before. */
    fun add(saved: List<RenderItem>, hidden: Set<String>) {
        for (item in saved) {
            if (item.objectId in hidden) continue
            val at = place.getOrPut(item.layerId) { place.size }
            if (at > topDrawn) topDrawn = at
            if (item.layerId !in first) first[item.layerId] = item
        }
    }

    fun copy(): DryLayers = DryLayers(LinkedHashMap(place), HashMap(first), topDrawn)

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
    /** Saved objects are being moved, resized or turned. */
    Preview(true),
    /** Saved objects are hidden while they are being erased. */
    Erasing(true),
}

/** Null when a frame may be drawn from a kept picture. [width] and [height]
 * are the view the scene was laid out for, [canvasWidth] and [canvasHeight]
 * the frame being drawn. */
internal fun pictureRefusal(hardware: Boolean, canvasWidth: Int, canvasHeight: Int, width: Int, height: Int, resizing: Boolean,
    giant: Boolean, documentTiles: Boolean, resultsReady: Boolean, results: Boolean,
    draft: Boolean, preview: Boolean, erasing: Boolean): PictureRefusal? = when {
    !hardware -> PictureRefusal.Software
    giant -> PictureRefusal.Giant
    documentTiles -> PictureRefusal.DocumentTiles
    !resultsReady || results -> PictureRefusal.Results
    resizing -> PictureRefusal.Resizing
    width <= 0 || height <= 0 || width != canvasWidth || height != canvasHeight -> PictureRefusal.Size
    draft -> PictureRefusal.Draft
    preview -> PictureRefusal.Preview
    erasing -> PictureRefusal.Erasing
    else -> null
}

/** Everything the pixels of a kept picture depend on. The picture holds the
 * saved document as it is stacked: the source pixels and every object layer,
 * transparent elsewhere. The surround, the white page, the page's edge, ink in
 * progress, selection, hover and the magnifier are drawn over or under it for
 * each frame, so the theme and those are not part of this.
 *
 * [objects] and [background] are compared by identity: a saved object is
 * never changed in place, and an edit, an undo, an erasure, a move, a change
 * from the paired computer and another document or page all arrive as other
 * objects. */
internal class PictureBasis<T>(val width: Int, val height: Int, val matrix: Transform, val scale: Double,
    val background: Any?, val smoothBackground: Boolean, val layerIds: List<String>, val hidden: Set<String>, val objects: List<T>)

/** What made a kept picture wrong. */
internal enum class PictureChange { Missing, Size, View, Background, Layers, Hidden, Objects }

internal sealed interface PictureStep {
    /** The picture is the saved scene. */
    data object Keep : PictureStep
    /** The picture is the saved scene without its objects from [from] on, and
     * those are drawn after everything else in a plainly composited layer:
     * they are drawn into the picture, and nothing else is. */
    data class Append(val from: Int) : PictureStep
    /** The picture is made again from every object. */
    data class Rebuild(val why: PictureChange) : PictureStep
}

/** What to do with a picture drawn from [held] so that it shows [next].
 * [layers] is what [DryLayers] holds for [held]'s objects. */
internal fun <T> pictureStep(held: PictureBasis<T>?, layers: DryLayers?, next: PictureBasis<T>, item: (T) -> RenderItem): PictureStep {
    if (held == null || layers == null) return PictureStep.Rebuild(PictureChange.Missing)
    if (held.width != next.width || held.height != next.height) return PictureStep.Rebuild(PictureChange.Size)
    if (held.matrix != next.matrix || held.scale != next.scale) return PictureStep.Rebuild(PictureChange.View)
    if (held.background !== next.background || held.smoothBackground != next.smoothBackground) return PictureStep.Rebuild(PictureChange.Background)
    if (held.layerIds != next.layerIds) return PictureStep.Rebuild(PictureChange.Layers)
    if (held.hidden !== next.hidden && held.hidden != next.hidden) return PictureStep.Rebuild(PictureChange.Hidden)
    if (held.objects === next.objects) return PictureStep.Keep
    val from = held.objects.size
    if (next.objects.size < from) return PictureStep.Rebuild(PictureChange.Objects)
    for (index in 0 until from) if (held.objects[index] !== next.objects[index]) return PictureStep.Rebuild(PictureChange.Objects)
    if (next.objects.size == from) return PictureStep.Keep
    val added = ArrayList<RenderItem>(next.objects.size - from)
    for (index in from until next.objects.size) added.add(item(next.objects[index]))
    return if (layers.inkOnTop(added, next.hidden)) PictureStep.Append(from) else PictureStep.Rebuild(PictureChange.Objects)
}

/** Whether a picture that has to be made again is made for this frame. It
 * is, unless the scene is changing from frame to frame, as it does while the
 * page is panned, zoomed or turned and while the paired computer moves an
 * object: then each frame would make a picture that the next one throws away.
 * Such a frame is drawn whole, as every frame used to be, and the picture is
 * made once the scene has stopped changing. [sameAsLastFrame]: the frame
 * before wanted this very picture. [sinceChangeMs]: how long ago a frame last
 * wanted another picture than the one before it. */
internal fun pictureMadeNow(sameAsLastFrame: Boolean, sinceChangeMs: Long): Boolean =
    sameAsLastFrame || sinceChangeMs < 0 || sinceChangeMs >= PICTURE_STREAK_MS

/** Changes closer together than this are one movement. */
internal const val PICTURE_STREAK_MS = 100L

/** How often each thing happened, for the on-device tests and nothing else. */
internal class PictureCounts {
    /** Pictures made from every object, pictures a stroke was added to, frames
     * drawn from a picture, frames drawn as a whole scene, and renders that failed. */
    val rebuilt = AtomicLong(); val added = AtomicLong(); val frames = AtomicLong(); val whole = AtomicLong(); val failed = AtomicLong()
    override fun toString(): String = "rebuilt=${rebuilt.get()} added=${added.get()} frames=${frames.get()} whole=${whole.get()} failed=${failed.get()}"
}

/** On-device tests only: one frame as the kept picture draws it and the same
 * scene drawn whole, both as plain pixels. */
internal class PictureComparison(val kept: Bitmap, val whole: Bitmap)

/** The kept picture of one render session, and the frames drawn from it.
 *
 * [draw] draws a whole frame when the scene allows it: it brings the picture
 * up to the saved scene first (nothing to do, one stroke to add, or all of it
 * again) and then draws the surround, the page, the picture, the ink in
 * progress, the overlays and the magnifier. When it returns false nothing was
 * drawn and the caller draws the whole scene as it always could.
 *
 * While the scene changes from frame to frame (a pan, a zoom, a turn, an
 * object moved from the paired computer) no picture is made for each frame:
 * those frames are drawn whole, and the picture is made when the movement
 * ends ([pictureMadeNow]).
 *
 * Memory: one screen-sized picture while the page rests. While strokes are
 * being added a second buffer is kept to draw the next state into, and at most
 * two more wait a moment to be released; [REST_MS] after the last change the
 * renderer and its spare buffers are given up and only the picture stays.
 * Everything goes at [forget] and [discard], and nothing is made after
 * [discard].
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

    private var surface: PictureSurface? = null
    private var held: Held? = null
    private var judged: Judged? = null
    private var layerIdsOf: DocumentSnapshot? = null
    private var layerIds: List<String> = emptyList()
    private var failures = 0
    private var discarded = false
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
    /** The movement has ended and no frame has come since: the picture is made for the next one. */
    private val settle = Runnable {
        synchronized(this) {
            val basis = wanted; val scene = wantedScene; val post = handler
            wanted = null; wantedScene = null
            if (!discarded && basis != null && scene != null && post != null && failures < MAX_FAILURES)
                replace(post, basis, judge(scene, basis.layerIds), counts.rebuilt) { EditorDrawing.drawStack(it, scene) }
        }
    }

    /** [steady]: no saved object is hidden for an erasure in progress.
     * [resizing]: the session owes the redraw of an in-place resize.
     * [first]: the session has not shown a frame yet. Its first frame is drawn
     * whole, so that a new canvas appears no later than it used to; the
     * picture is made right after, before a pen can need it. */
    @Synchronized fun draw(canvas: Canvas, scene: CanvasScene, steady: Boolean, resizing: Boolean, first: Boolean): Boolean {
        if (discarded) return false
        if (dropAsked) { dropAsked = false; forget() }
        val width = scene.camera.viewportWidth.toInt(); val height = scene.camera.viewportHeight.toInt()
        val refusal = pictureRefusal(canvas.isHardwareAccelerated, canvas.width, canvas.height, width, height, resizing,
            giant = scene.giantMode || scene.giantFrame != null, documentTiles = scene.documentFrame != null,
            resultsReady = scene.resultDisplayReady, results = scene.aiResults.isNotEmpty(), draft = scene.documentDraft != null,
            preview = scene.transformed.isNotEmpty(), erasing = !steady)
        if (refusal != null) { if (!refusal.keepsPicture) forget(); counts.whole.incrementAndGet(); return false }
        // Releasing a replaced picture is posted to this thread, behind every frame drawn before.
        val looper = Looper.myLooper()
        if (looper == null || failures >= MAX_FAILURES) { counts.whole.incrementAndGet(); return false }
        val post = handler?.takeIf { it.looper === looper } ?: Handler(looper).also { handler = it }
        val ids = if (layerIdsOf === scene.document) layerIds else scene.document.layers.map { it.id }.also { layerIds = it; layerIdsOf = scene.document }
        val layers = judge(scene, ids)
        if (scene.provisional.isNotEmpty() && !layers.inkOnTop(scene.provisional.map { it.item }, scene.hidden)) {
            counts.whole.incrementAndGet(); return false
        }
        val next = PictureBasis(width, height, scene.matrix, scene.camera.scale, scene.background, scene.smoothBackground, ids, scene.hidden, scene.objects)
        val now = held
        val step = pictureStep(now?.basis, now?.layers, next) { it.item }
        if (step is PictureStep.Rebuild) {
            val uptime = SystemClock.uptimeMillis()
            val same = wanted?.let { pictureStep(it, layers, next) { drawn -> drawn.item } == PictureStep.Keep } == true
            val made = !first && pictureMadeNow(same, if (changedMs == 0L) -1 else uptime - changedMs)
            if (!same) changedMs = uptime
            if (!made) {
                // Drawn whole for now; made when the next frame wants the same, or when none comes.
                wanted = next; wantedScene = scene
                post.removeCallbacks(settle); post.postDelayed(settle, PICTURE_SETTLE_MS)
                counts.whole.incrementAndGet(); return false
            }
        }
        post.removeCallbacks(settle); wanted = null; wantedScene = null
        val picture = when (step) {
            PictureStep.Keep -> checkNotNull(now).also { if (it.basis.objects !== next.objects) held = Held(it.picture, next, it.layers) }.picture
            is PictureStep.Append -> {
                val before = checkNotNull(now).picture.bitmap
                replace(post, next, layers, counts.added) { EditorDrawing.drawAdded(it, scene, before, scene.objects.subList(step.from, scene.objects.size)) }
            }
            is PictureStep.Rebuild -> replace(post, next, layers, counts.rebuilt) { EditorDrawing.drawStack(it, scene) }
        } ?: return false
        EditorDrawing.drawKept(canvas, scene, picture.bitmap)
        PrecisionDrawing.drawPanes(canvas, scene, onlyVisible = true)
        counts.frames.incrementAndGet()
        return true
    }

    /** The layer order for [scene]'s saved objects: the one held, the one held
     * with the objects added at the end, or a new one. */
    private fun judge(scene: CanvasScene, ids: List<String>): DryLayers {
        val last = judged
        val objects = scene.objects
        if (last != null && last.layerIds == ids && (last.hidden === scene.hidden || last.hidden == scene.hidden)) {
            if (last.objects === objects) return last.layers
            val from = last.objects.size
            var extends = objects.size >= from
            var index = 0
            while (extends && index < from) { if (last.objects[index] !== objects[index]) extends = false; index++ }
            if (extends) {
                val added = ArrayList<RenderItem>(objects.size - from)
                for (at in from until objects.size) added.add(objects[at].item)
                // A held picture has its own copy of what it was drawn from.
                last.layers.add(added, scene.hidden)
                judged = Judged(objects, scene.hidden, ids, last.layers)
                return last.layers
            }
        }
        return DryLayers(ids, objects.map { it.item }, scene.hidden).also { judged = Judged(objects, scene.hidden, ids, it) }
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
        val made = target.render(draw) ?: return failed()
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
        val pictures = try { PictureSurface(now.basis.width, now.basis.height) } catch (error: RuntimeException) { return null }
        try {
            fun pixels(draw: (Canvas) -> Unit): Bitmap? {
                val made = pictures.render(draw) ?: return null
                return try { made.bitmap.copy(Bitmap.Config.ARGB_8888, false) } finally { made.release() }
            }
            val kept = pixels { EditorDrawing.drawKept(it, scene, now.picture.bitmap); PrecisionDrawing.drawPanes(it, scene, onlyVisible = true) } ?: return null
            val whole = pixels { PrecisionDrawing.draw(it, scene) } ?: run { kept.recycle(); return null }
            return PictureComparison(kept, whole)
        } finally { pictures.close() }
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
        wanted = null; wantedScene = null
        surface?.close(); surface = null
        held?.picture?.release(); held = null
        replaced.forEach { it.release() }; replaced.clear()
        judged = null; layerIdsOf = null; layerIds = emptyList()
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
    }
}
