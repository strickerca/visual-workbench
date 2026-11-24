package com.visualworkbench.android.editor

import com.visualworkbench.shared.Rect
import com.visualworkbench.shared.RenderItem
import com.visualworkbench.shared.Shape
import com.visualworkbench.shared.Transform
import java.util.IdentityHashMap
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.floor

// Undo, redo, an erasure, a move, a turn, a deletion, a restyle and a change
// from the paired computer each touch a small part of the page. The kept
// picture is brought up to date by drawing that part again, and while a
// stroke is being moved or turned each frame draws that part over the
// picture. These are the rules: what changed between two lists of saved
// objects, where on the screen it is drawn, and how the saved objects that
// can reach a part of the page are found without a pass over all of them.
// Plain functions and classes, tested without a phone.

/** A rectangle of whole screen pixels: [left, right) x [top, bottom). */
internal data class PixelBox(val left: Int, val top: Int, val right: Int, val bottom: Int) {
    val pixels: Long get() = (right - left).toLong() * (bottom - top)
    fun meets(other: PixelBox): Boolean = left <= other.right && other.left <= right && top <= other.bottom && other.top <= bottom
    fun join(other: PixelBox): PixelBox = PixelBox(minOf(left, other.left), minOf(top, other.top), maxOf(right, other.right), maxOf(bottom, other.bottom))
}

/** The part of a [width] x [height] screen that a change touched, as a few
 * pixel boxes no two of which share a pixel. A box that overlaps or touches
 * another is joined with it into the box around both, and at most [MAX_BOXES]
 * are kept (beyond that, the two whose joined box adds least are joined): each
 * box can then be drawn again by itself, and no pixel is drawn twice.
 * [unknown]: something touched could not be placed on the screen. */
internal class TouchedArea(val width: Int, val height: Int) {
    private val held = ArrayList<PixelBox>(4)
    val boxes: List<PixelBox> get() = held
    var unknown = false
        private set
    val pixels: Long get() { var sum = 0L; for (box in held) sum += box.pixels; return sum }
    val isEmpty: Boolean get() = !unknown && held.isEmpty()
    /** Drawing the touched part again would cost about as much as the whole
     * screen: it is more than half of it, or it is not known. */
    val large: Boolean get() = unknown || pixels * 2 > width.toLong() * height

    /** Adds the screen box from ([left], [top]) to ([right], [bottom]), in
     * pixels with any fraction, grown out to whole pixels and cut to the screen. */
    fun add(left: Double, top: Double, right: Double, bottom: Double) {
        if (!(left.isFinite() && top.isFinite() && right.isFinite() && bottom.isFinite()) || right < left || bottom < top) { unknown = true; return }
        val l = floor(left).coerceIn(0.0, width.toDouble()).toInt(); val t = floor(top).coerceIn(0.0, height.toDouble()).toInt()
        val r = ceil(right).coerceIn(0.0, width.toDouble()).toInt(); val b = ceil(bottom).coerceIn(0.0, height.toDouble()).toInt()
        if (l >= r || t >= b) return
        insert(PixelBox(l, t, r, b))
        while (held.size > MAX_BOXES) joinCheapest()
    }

    /** Takes [box] in, joined with every box it meets, and those with every box they then meet. */
    private fun insert(box: PixelBox) {
        var grown = box
        var joined = true
        while (joined) {
            joined = false
            val each = held.iterator()
            while (each.hasNext()) { val other = each.next(); if (other.meets(grown)) { grown = grown.join(other); each.remove(); joined = true } }
        }
        held.add(grown)
    }

    private fun joinCheapest() {
        var best = 0; var second = 1; var cost = Long.MAX_VALUE
        for (i in held.indices) for (j in i + 1 until held.size) {
            val added = held[i].join(held[j]).pixels - held[i].pixels - held[j].pixels
            if (added < cost) { cost = added; best = i; second = j }
        }
        val joined = held[best].join(held[second])
        held.removeAt(second); held.removeAt(best)
        insert(joined)
    }

    override fun toString(): String = if (unknown) "unknown" else "pixels=$pixels boxes=${held.size}"

    companion object {
        /** Boxes drawn again at most: each costs a pass over the objects found for it. */
        const val MAX_BOXES = 6
    }
}

/** How far beyond its native box an object's pixels may reach on the screen
 * (its anti-aliased edge, and rounding of the matrices drawn with). */
internal const val TOUCH_MARGIN_PX = 2.0

/** Whether everything the canvas draws for [item] lies within its native box,
 * so that a change to it can be placed on the screen: strokes, lines, arrows,
 * polygons, rectangles, ellipses, text and guides. (The box of a stroke is its
 * outline's; a shape's is grown by twice its width, more than the half width
 * its outline reaches, and an arrow's by more than its head.) A numbered
 * marker draws a label laid out on the phone, and an AI result pixels of its
 * own: a change to either is not placed, and the whole picture is made again. */
internal fun boxHoldsDrawing(item: RenderItem): Boolean = when (item.shape) {
    is Shape.Stroke, is Shape.Line, is Shape.Arrow, is Shape.Polygon, is Shape.Rectangle, is Shape.Ellipse, is Shape.Text, is Shape.Guide -> true
    else -> false
}

/** The inverse of [t], or null when it has none. */
internal fun invert(t: Transform): Transform? {
    val determinant = t.a * t.d - t.b * t.c
    if (!determinant.isFinite() || abs(determinant) < 1e-18) return null
    return Transform(t.d / determinant, -t.b / determinant, -t.c / determinant, t.a / determinant,
        (t.c * t.f - t.d * t.e) / determinant, (t.b * t.e - t.a * t.f) / determinant)
}

/** Adds to [area] where [item] is drawn on the screen: its native box (which
 * already holds its own transform) under [view] (document to screen), moved
 * by [preview] in place of its own transform when one is given, grown by
 * [TOUCH_MARGIN_PX] and [extra] pixels (where it shows in a picture drawn
 * smoothed) and, for an outline of constant screen width, by that width.
 * False: it cannot be placed ([boxHoldsDrawing], a box or transform that
 * cannot be judged). */
internal fun addDrawn(area: TouchedArea, item: RenderItem, view: Transform, preview: Transform?, extra: Double = 0.0): Boolean {
    val box = drawnBox(item, view, preview, extra) ?: return false
    area.add(box[0], box[1], box[2], box[3])
    return true
}

/** Where [item] is drawn on the screen, as [addDrawn] places it: left, top,
 * right and bottom in pixels with any fraction; null when it cannot be placed. */
internal fun drawnBox(item: RenderItem, view: Transform, preview: Transform?, extra: Double = 0.0): DoubleArray? {
    if (!boxHoldsDrawing(item)) return null
    val b = item.bounds
    if (!(b.x.isFinite() && b.y.isFinite() && b.width.isFinite() && b.height.isFinite()) || b.width < 0 || b.height < 0) return null
    val full = if (preview == null || preview == item.transform) view else {
        val inverse = invert(item.transform) ?: return null
        EditorDrawing.compose(view, EditorDrawing.compose(preview, inverse))
    }
    var minX = Double.POSITIVE_INFINITY; var minY = Double.POSITIVE_INFINITY
    var maxX = Double.NEGATIVE_INFINITY; var maxY = Double.NEGATIVE_INFINITY
    for (corner in 0 until 4) {
        val x = if (corner == 1 || corner == 2) b.x + b.width else b.x
        val y = if (corner >= 2) b.y + b.height else b.y
        val sx = full.a * x + full.c * y + full.e; val sy = full.b * x + full.d * y + full.f
        if (sx < minX) minX = sx; if (sx > maxX) maxX = sx
        if (sy < minY) minY = sy; if (sy > maxY) maxY = sy
    }
    if (!(minX.isFinite() && minY.isFinite() && maxX.isFinite() && maxY.isFinite())) return null
    val margin = TOUCH_MARGIN_PX + maxOf(0.0, extra) + if (item.style.screenConstantWidth && item.style.width.isFinite()) maxOf(0.0, item.style.width) else 0.0
    return doubleArrayOf(minX - margin, minY - margin, maxX + margin, maxY + margin)
}

/** Grows [area] until no object it is drawn again from is cut by the edge of
 * one of its boxes, for at most [rounds] rounds: the renderer smooths a path
 * that a clip cuts otherwise than one that lies within, so a part drawn again
 * is the whole scene's pixels only where nothing in it is cut. [near] names
 * the objects that could reach a screen box (with where each is drawn under
 * [view], moved by a preview when [preview] gives one). Objects that cannot be
 * placed (a numbered marker, an AI result) are drawn as they are. Stops early
 * once the area is large ([TouchedArea.large]). */
internal fun <T> closeArea(area: TouchedArea, view: Transform, near: (PixelBox) -> List<T>?, item: (T) -> RenderItem, preview: (T) -> Transform?,
                           extra: Double = 0.0, rounds: Int = CLOSE_ROUNDS) {
    repeat(rounds) {
        if (area.isEmpty || area.large) return
        val before = area.boxes.toList()
        for (box in before) {
            val found = near(box) ?: run { area.add(Double.NaN, 0.0, 0.0, 0.0); return }
            for (one in found) {
                val drawn = drawnBox(item(one), view, preview(one), extra) ?: continue
                // Cut by this box: it reaches in, and does not lie within.
                val reaches = drawn[0] < box.right && drawn[2] > box.left && drawn[1] < box.bottom && drawn[3] > box.top
                val within = drawn[0] >= box.left && drawn[2] <= box.right && drawn[1] >= box.top && drawn[3] <= box.bottom
                if (reaches && !within) area.add(drawn[0], drawn[1], drawn[2], drawn[3])
            }
        }
        if (area.boxes == before) return
    }
}

/** Rounds [closeArea] grows an area by at most. */
internal const val CLOSE_ROUNDS = 4

/** What differs between two lists of saved objects in drawing order, by
 * identity (a saved object is never changed in place: an edit arrives as
 * another object). [removed] are positions in the old list, [added] positions
 * in the new one, both ascending. An object in both lists whose place among
 * the others changed is in both. Every other object is in both lists, in the
 * same order. */
internal class ObjectsDiff(val removed: IntArray, val added: IntArray) {
    val size: Int get() = removed.size + added.size
    val isEmpty: Boolean get() = removed.isEmpty() && added.isEmpty()
    /** Whether the new list is the old one with objects added after all of it. */
    fun appended(oldSize: Int, newSize: Int): Boolean =
        removed.isEmpty() && added.size == newSize - oldSize && added.isNotEmpty() && added[0] == oldSize

    companion object { val NONE = ObjectsDiff(IntArray(0), IntArray(0)) }
}

/** The difference between [old] and [new]; null when more than [limit]
 * objects differ, or when one list holds the same object twice. The lists'
 * common start and end are passed over by identity, or by [same] when it is
 * given (objects it takes for one another count as the same there); what lies
 * between is matched by identity, and of the objects in both, the longest run
 * that kept its order is kept. The cost is one look at each object of the
 * common start and end, and more only for the part that changed. */
internal fun <T : Any> objectsDiff(old: List<T>, new: List<T>, limit: Int, same: ((T, T) -> Boolean)? = null): ObjectsDiff? {
    if (old === new) return ObjectsDiff.NONE
    val shorter = minOf(old.size, new.size)
    var start = 0
    while (start < shorter && (old[start] === new[start] || same?.invoke(old[start], new[start]) == true)) start++
    var oldEnd = old.size; var newEnd = new.size
    while (oldEnd > start && newEnd > start && (old[oldEnd - 1] === new[newEnd - 1] || same?.invoke(old[oldEnd - 1], new[newEnd - 1]) == true)) { oldEnd--; newEnd-- }
    val oldCount = oldEnd - start; val newCount = newEnd - start
    if (oldCount == 0 && newCount == 0) return ObjectsDiff.NONE
    if (oldCount == 0 || newCount == 0) {
        if (oldCount + newCount > limit) return null
        return ObjectsDiff(IntArray(oldCount) { start + it }, IntArray(newCount) { start + it })
    }
    if (oldCount > limit + newCount || newCount > limit + oldCount) return null
    val placeInNew = IdentityHashMap<T, Int>(newCount)
    for (at in start until newEnd) if (placeInNew.put(new[at], at) != null) return null
    // For each object of the old middle, its place in the new list (-1: gone).
    val places = IntArray(oldCount)
    val taken = BooleanArray(newCount)
    for (k in 0 until oldCount) {
        val place = placeInNew[old[start + k]]
        if (place == null) places[k] = -1
        else { if (taken[place - start]) return null; taken[place - start] = true; places[k] = place }
    }
    val kept = longestIncreasing(places)
    var removedCount = 0
    for (k in 0 until oldCount) if (!kept[k]) removedCount++
    val keptNew = BooleanArray(newCount)
    for (k in 0 until oldCount) if (kept[k]) keptNew[places[k] - start] = true
    var addedCount = 0
    for (k in 0 until newCount) if (!keptNew[k]) addedCount++
    if (removedCount + addedCount > limit) return null
    val removed = IntArray(removedCount); var r = 0
    for (k in 0 until oldCount) if (!kept[k]) removed[r++] = start + k
    val added = IntArray(addedCount); var a = 0
    for (k in 0 until newCount) if (!keptNew[k]) added[a++] = start + k
    return ObjectsDiff(removed, added)
}

/** Which entries of [values] (non-negative, distinct; -1 is skipped) form one
 * longest strictly increasing run. */
internal fun longestIncreasing(values: IntArray): BooleanArray {
    val n = values.size
    val tails = IntArray(n) // index of the smallest tail of a run of each length
    val before = IntArray(n) { -1 }
    var length = 0
    for (i in 0 until n) {
        val v = values[i]
        if (v < 0) continue
        var low = 0; var high = length
        while (low < high) { val mid = (low + high) ushr 1; if (values[tails[mid]] < v) low = mid + 1 else high = mid }
        if (low > 0) before[i] = tails[low - 1]
        tails[low] = i
        if (low == length) length++
    }
    val kept = BooleanArray(n)
    var at = if (length == 0) -1 else tails[length - 1]
    while (at >= 0) { kept[at] = true; at = before[at] }
    return kept
}

/** The saved objects of one list placed on a coarse grid over the page by
 * their native boxes, so that a part of the page finds the objects that could
 * reach it without a pass over all of them: a magnifier pane, which shows a
 * small part of the page, and the part of the screen a change touched.
 * Objects that may not be left out by their box ([cullable] false), whose box
 * cannot be judged, or that would cover many cells are always named.
 *
 * Each object has an order key that grows along the list, and what is found
 * is returned in list order. The index follows its list as it changes
 * ([follow]): an object taken out is taken off the grid, and an object put in
 * is placed with a key between those of its neighbours, so that a change
 * costs as much as the objects it changed, not as much as the list. Objects
 * are told apart by identity, and found by [id]. */
internal class SavedIndex<T : Any>(val pageWidth: Double, val pageHeight: Double, private val box: (T) -> Rect,
                                   private val cullable: (T) -> Boolean, private val id: (T) -> String) {
    private val cellWidth = (if (pageWidth.isFinite() && pageWidth > 0) pageWidth / CELLS else 1.0).coerceAtLeast(1.0)
    private val cellHeight = (if (pageHeight.isFinite() && pageHeight > 0) pageHeight / CELLS else 1.0).coerceAtLeast(1.0)
    private val cells = arrayOfNulls<ArrayList<T>>(CELLS * CELLS)
    private val keys = IdentityHashMap<T, Double>()
    private val always = IdentityHashMap<T, Boolean>()
    private val byId = HashMap<String, T>()
    /** The list this describes. */
    var list: List<T> = emptyList()
        private set
    /** Times the keys ran out of room between two neighbours and were given out again. */
    var renumbered = 0
        private set
    val size: Int get() = keys.size

    /** Describes [objects] from nothing. */
    fun reset(objects: List<T>) {
        cells.fill(null); keys.clear(); always.clear(); byId.clear()
        for (at in objects.indices) place(objects[at], at.toDouble())
        list = objects
    }

    /** Describes [next], which differs from [list] by [diff] ([objectsDiff]). */
    fun follow(next: List<T>, diff: ObjectsDiff) {
        val old = list
        for (at in diff.removed) unplace(old[at])
        val added = diff.added
        var i = 0
        var crowded = false
        while (i < added.size) {
            var j = i
            while (j + 1 < added.size && added[j + 1] == added[j] + 1) j++
            val first = added[i]; val last = added[j]; val count = j - i + 1
            // The neighbours of a run of new objects are objects that stayed,
            // which have keys; were one missing, all keys are given out again.
            val low = if (first == 0) null else keys[next[first - 1]] ?: run { crowded = true; null }
            val high = if (last == next.size - 1) null else keys[next[last + 1]] ?: run { crowded = true; null }
            // Each key above the one before it (the low neighbour's, then the
            // last one given) and below the high neighbour's; when the room
            // between them is too small for that, all keys are given out again.
            var before = low
            for (k in 0 until count) {
                val key = when {
                    low == null && high == null -> k.toDouble()
                    high == null -> low!! + 1.0 + k
                    low == null -> high - count + k
                    else -> low + (high - low) * (k + 1) / (count + 1)
                }
                if ((before != null && !(key > before)) || (high != null && !(key < high))) crowded = true
                place(next[first + k], key)
                before = key
            }
            i = j + 1
        }
        list = next
        if (crowded) { for (at in next.indices) keys[next[at]] = at.toDouble(); renumbered++ }
    }

    /** The object of the list with [objectId], if any. */
    fun find(objectId: String): T? = byId[objectId]

    /** Every object whose box could reach [visible] grown by [margin] on each
     * side, every object always named, and [extra] (objects of the list), in
     * list order, each once. Null when [visible] cannot be judged: then all of
     * them could. */
    fun near(visible: Rect, margin: Double, extra: Collection<T> = emptyList()): List<T>? {
        if (!(margin >= 0.0) || !margin.isFinite() || !visible.x.isFinite() || !visible.y.isFinite() || !visible.width.isFinite() || !visible.height.isFinite()) return null
        val x0 = column(visible.x - margin); val x1 = column(visible.x + visible.width + margin)
        val y0 = row(visible.y - margin); val y1 = row(visible.y + visible.height + margin)
        val found = ArrayList<T>()
        found.addAll(always.keys)
        for (item in extra) if (keys.containsKey(item)) found.add(item)
        for (y in y0..y1) for (x in x0..x1) cells[y * CELLS + x]?.let(found::addAll)
        val ordered = arrayOfNulls<Keyed<T>>(found.size)
        // Every object on the grid has a key; one without could not be put in order, and then all have to be looked at.
        for (at in found.indices) { val item = found[at]; ordered[at] = Keyed(keys[item] ?: return null, item) }
        ordered.sortWith(compareBy { it!!.key })
        val result = ArrayList<T>(ordered.size)
        var last: T? = null
        for (entry in ordered) { val item = entry!!.item; if (item !== last) result.add(item); last = item }
        return result
    }
    private class Keyed<T>(val key: Double, val item: T)

    /** On-device tests and unit tests only: the key of [item], if it is in the list. */
    fun keyOf(item: T): Double? = keys[item]

    private fun place(item: T, key: Double) {
        keys[item] = key
        byId[id(item)] = item
        val range = cellsOf(item)
        if (range == null) { always[item] = true; return }
        for (y in range[2]..range[3]) for (x in range[0]..range[1]) (cells[y * CELLS + x] ?: ArrayList<T>(4).also { cells[y * CELLS + x] = it }).add(item)
    }

    private fun unplace(item: T) {
        if (keys.remove(item) == null) return
        val name = id(item)
        if (byId[name] === item) byId.remove(name)
        if (always.remove(item) != null) return
        val range = cellsOf(item) ?: return
        for (y in range[2]..range[3]) for (x in range[0]..range[1]) {
            val cell = cells[y * CELLS + x] ?: continue
            for (at in cell.indices) if (cell[at] === item) { cell[at] = cell[cell.size - 1]; cell.removeAt(cell.size - 1); break }
        }
    }

    /** The cells [item]'s box covers (columns, then rows), or null when it is always named. */
    private fun cellsOf(item: T): IntArray? {
        val bounds = box(item)
        if (!cullable(item) || !bounds.x.isFinite() || !bounds.y.isFinite() || !bounds.width.isFinite() || !bounds.height.isFinite() ||
            bounds.width < 0 || bounds.height < 0) return null
        val x0 = column(bounds.x); val x1 = column(bounds.x + bounds.width)
        val y0 = row(bounds.y); val y1 = row(bounds.y + bounds.height)
        if ((x1 - x0 + 1) * (y1 - y0 + 1) > MAX_CELLS) return null
        return intArrayOf(x0, x1, y0, y1)
    }

    // Monotonic, and clamped to the grid: a box off the page lands in the edge
    // cells, as does any part of an area off the page, so overlaps are kept.
    private fun column(x: Double): Int = floor(x / cellWidth).coerceIn(0.0, (CELLS - 1).toDouble()).toInt()
    private fun row(y: Double): Int = floor(y / cellHeight).coerceIn(0.0, (CELLS - 1).toDouble()).toInt()

    private companion object {
        /** Cells across and down the page. */
        const val CELLS = 32
        /** An object covering more cells than this is always named. */
        const val MAX_CELLS = 64
    }
}
