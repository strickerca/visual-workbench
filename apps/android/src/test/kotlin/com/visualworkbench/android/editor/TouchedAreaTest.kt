@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

/** The rules of drawing a part of the page again: what changed between two
 * lists of saved objects, where on the screen that is, how the screen boxes
 * are joined, when a part is drawn again and when the whole picture is made
 * instead, and which saved objects can reach a part of the page. The drawing
 * itself is judged on the device (KeptPictureInstrumentedTest). */
class TouchedAreaTest {
    private fun item(id: String, layer: String = "notes", bounds: Rect = Rect(0.0, 0.0, 1.0, 1.0), shape: Shape = Shape.Stroke("pen"),
                     style: ObjectStyle = ObjectStyle(0x203040ffu, 4.0), transform: Transform = Transform(), opacity: Double = 1.0, blend: String = "normal") =
        RenderItem(id, layer, opacity, blend, bounds, Contours(longArrayOf(), longArrayOf(), uintArrayOf()), transform, style, shape, false)

    // region Screen boxes
    @Test fun a_box_is_grown_out_to_whole_pixels_and_cut_to_the_screen() {
        val area = TouchedArea(100, 80)
        area.add(10.2, 20.7, 30.1, 40.0)
        assertEquals(listOf(PixelBox(10, 20, 31, 40)), area.boxes)
        assertEquals(21L * 20, area.pixels)
        // Partly off the screen: cut to it. Wholly off it, or empty: nothing.
        val edge = TouchedArea(100, 80).apply { add(-15.0, 70.5, 5.0, 300.0) }
        assertEquals(listOf(PixelBox(0, 70, 5, 80)), edge.boxes)
        val off = TouchedArea(100, 80).apply { add(150.0, 10.0, 160.0, 20.0); add(10.0, -40.0, 20.0, -1.0); add(30.0, 30.0, 30.0, 30.0) }
        assertTrue(off.isEmpty); assertFalse(off.large)
    }
    @Test fun a_box_that_cannot_be_judged_makes_the_whole_screen_touched() {
        for (bad in listOf(doubleArrayOf(Double.NaN, 0.0, 1.0, 1.0), doubleArrayOf(0.0, 0.0, Double.POSITIVE_INFINITY, 1.0), doubleArrayOf(5.0, 0.0, 1.0, 1.0))) {
            val area = TouchedArea(100, 80).apply { add(bad[0], bad[1], bad[2], bad[3]) }
            assertTrue(area.unknown); assertTrue(area.large); assertFalse(area.isEmpty)
        }
    }
    @Test fun boxes_that_overlap_or_touch_are_joined_and_none_shares_a_pixel() {
        val area = TouchedArea(1000, 1000)
        area.add(0.0, 0.0, 10.0, 10.0)
        area.add(10.0, 0.0, 20.0, 10.0)   // touches: joined
        assertEquals(listOf(PixelBox(0, 0, 20, 10)), area.boxes)
        area.add(100.0, 100.0, 110.0, 110.0)
        assertEquals(2, area.boxes.size)
        // A box over both joins all three, and the joined box is joined again with what it now meets.
        area.add(200.0, 200.0, 210.0, 210.0)
        area.add(5.0, 5.0, 105.0, 105.0)
        assertEquals(listOf(PixelBox(200, 200, 210, 210), PixelBox(0, 0, 110, 110)).toSet(), area.boxes.toSet())
        val random = java.util.Random(3)
        repeat(200) {
            val many = TouchedArea(1080, 2000)
            repeat(1 + random.nextInt(30)) {
                val x = random.nextDouble() * 1200 - 60; val y = random.nextDouble() * 2100 - 50
                many.add(x, y, x + random.nextDouble() * 200, y + random.nextDouble() * 200)
            }
            assertTrue(many.boxes.size <= TouchedArea.MAX_BOXES)
            for (i in many.boxes.indices) for (j in i + 1 until many.boxes.size) {
                val p = many.boxes[i]; val q = many.boxes[j]
                assertFalse("$p and $q share pixels", p.left < q.right && q.left < p.right && p.top < q.bottom && q.top < p.bottom)
            }
            for (box in many.boxes) assertTrue(box.left >= 0 && box.top >= 0 && box.right <= 1080 && box.bottom <= 2000 && box.left < box.right && box.top < box.bottom)
        }
    }
    @Test fun at_most_a_few_boxes_are_kept_the_cheapest_are_joined() {
        val area = TouchedArea(2000, 2000)
        // Six boxes in a row 300 apart, and one more close to the last: that pair is joined first.
        for (k in 0 until TouchedArea.MAX_BOXES) area.add(k * 300.0, 0.0, k * 300.0 + 10, 10.0)
        area.add(1530.0, 0.0, 1540.0, 10.0)
        assertEquals(TouchedArea.MAX_BOXES, area.boxes.size)
        assertTrue(area.boxes.contains(PixelBox(1500, 0, 1540, 10)))
    }
    @Test fun more_than_half_the_screen_is_large() {
        assertFalse(TouchedArea(100, 100).apply { add(0.0, 0.0, 50.0, 100.0) }.large)
        assertTrue(TouchedArea(100, 100).apply { add(0.0, 0.0, 51.0, 100.0) }.large)
        assertTrue(TouchedArea(100, 100).apply { add(0.0, 0.0, 40.0, 100.0); add(60.0, 0.0, 71.0, 100.0) }.large)
    }
    // endregion

    // region Where an object is drawn
    private val view = Transform(a = 2.0, d = 2.0, e = 10.0, f = 20.0)

    @Test fun an_object_is_placed_by_its_box_under_the_view_grown_by_the_margin() {
        val area = TouchedArea(1000, 1000)
        assertTrue(addDrawn(area, item("a", bounds = Rect(10.0, 10.0, 5.0, 4.0)), view, null))
        // (30, 40)..(40, 48), grown by two pixels.
        assertEquals(listOf(PixelBox(28, 38, 42, 50)), area.boxes)
    }
    @Test fun an_object_being_moved_or_turned_is_placed_where_it_is_shown() {
        // Its box already holds its own transform; the preview replaces that transform.
        val own = Transform(e = 100.0)
        val moved = item("m", bounds = Rect(100.0, 0.0, 10.0, 10.0), transform = own)
        val area = TouchedArea(2000, 2000)
        assertTrue(addDrawn(area, moved, Transform(), Transform(e = 300.0, f = 50.0)))
        assertEquals(listOf(PixelBox(298, 48, 312, 62)), area.boxes)
        // A quarter turn about the origin: the box of the turned box.
        val turned = TouchedArea(2000, 2000)
        assertTrue(addDrawn(turned, item("t", bounds = Rect(100.0, 0.0, 10.0, 20.0)), Transform(e = 500.0, f = 500.0), Transform(a = 0.0, b = 1.0, c = -1.0, d = 0.0)))
        assertEquals(listOf(PixelBox(478, 598, 502, 612)), turned.boxes)
        // A preview equal to the object's own transform is no move.
        val same = TouchedArea(2000, 2000)
        assertTrue(addDrawn(same, moved, Transform(), own))
        assertEquals(listOf(PixelBox(98, 0, 112, 12)), same.boxes)
    }
    @Test fun an_outline_of_constant_screen_width_reaches_that_much_further() {
        val area = TouchedArea(1000, 1000)
        val line = item("l", bounds = Rect(10.0, 10.0, 5.0, 4.0), shape = Shape.Line(listOf(Point(10.0, 10.0), Point(15.0, 14.0))),
            style = ObjectStyle(0x203040ffu, 6.0, screenConstantWidth = true))
        assertTrue(addDrawn(area, line, view, null))
        assertEquals(listOf(PixelBox(22, 32, 48, 56)), area.boxes)
    }
    @Test fun only_objects_whose_box_holds_their_drawing_are_placed() {
        val box = Rect(0.0, 0.0, 10.0, 10.0)
        for (shape in listOf(Shape.Stroke("pen"), Shape.Line(emptyList()), Shape.Arrow(emptyList()), Shape.Polygon(emptyList(), true),
                Shape.Rectangle(box), Shape.Ellipse(box), Shape.Text(Point(0.0, 0.0), "a", "Inter", 12.0), Shape.Guide(box))) {
            assertTrue(shape.toString(), boxHoldsDrawing(item("x", shape = shape)))
            assertTrue(shape.toString(), addDrawn(TouchedArea(100, 100), item("x", bounds = box, shape = shape), Transform(), null))
        }
        for (shape in listOf(Shape.Marker(1u, Point(0.0, 0.0), null), Shape.Result("asset", "result"), Shape.Adjustment)) {
            assertFalse(shape.toString(), boxHoldsDrawing(item("x", shape = shape)))
            assertFalse(shape.toString(), addDrawn(TouchedArea(100, 100), item("x", bounds = box, shape = shape), Transform(), null))
        }
        // A box or a transform that cannot be judged.
        assertFalse(addDrawn(TouchedArea(100, 100), item("x", bounds = Rect(Double.NaN, 0.0, 1.0, 1.0)), Transform(), null))
        assertFalse(addDrawn(TouchedArea(100, 100), item("x", bounds = Rect(0.0, 0.0, -1.0, 1.0)), Transform(), null))
        assertFalse(addDrawn(TouchedArea(100, 100), item("x", transform = Transform(a = 0.0, d = 0.0)), Transform(), Transform(e = 4.0)))
        assertNull(invert(Transform(a = 0.0, b = 0.0, c = 0.0, d = 0.0)))
        val t = Transform(a = 2.0, b = 0.5, c = -0.3, d = 1.5, e = 7.0, f = -3.0)
        val back = EditorDrawing.compose(t, requireNotNull(invert(t)))
        for ((got, want) in listOf(back.a to 1.0, back.b to 0.0, back.c to 0.0, back.d to 1.0, back.e to 0.0, back.f to 0.0)) assertEquals(want, got, 1e-12)
    }
    // endregion

    // region What changed between two lists
    /** Applying [diff] to [old] gives [new]: what stays keeps its order. */
    private fun <T : Any> check(old: List<T>, new: List<T>, diff: ObjectsDiff) {
        val removed = diff.removed.toSet(); val added = diff.added.toSet()
        val stayed = old.filterIndexed { at, _ -> at !in removed }
        val kept = new.filterIndexed { at, _ -> at !in added }
        assertEquals(stayed.size, kept.size)
        for (at in stayed.indices) assertSame(stayed[at], kept[at])
        for (at in diff.removed.indices.drop(1)) assertTrue(diff.removed[at - 1] < diff.removed[at])
        for (at in diff.added.indices.drop(1)) assertTrue(diff.added[at - 1] < diff.added[at])
    }

    @Test fun lists_are_told_apart_by_identity() {
        val a = item("a"); val b = item("b"); val c = item("c"); val d = item("d")
        assertTrue(requireNotNull(objectsDiff(listOf(a, b), listOf(a, b), 10)).isEmpty)
        // The same objects in other lists are no change; another object with the same id is.
        val other = item("b")
        val swapped = requireNotNull(objectsDiff(listOf(a, b, c), listOf(a, other, c), 10))
        assertArrayEquals(intArrayOf(1), swapped.removed); assertArrayEquals(intArrayOf(1), swapped.added)
        // Added at the end, taken from the middle, put in at the start.
        val appended = requireNotNull(objectsDiff(listOf(a, b), listOf(a, b, c, d), 10))
        assertTrue(appended.appended(2, 4)); assertArrayEquals(intArrayOf(2, 3), appended.added)
        val taken = requireNotNull(objectsDiff(listOf(a, b, c), listOf(a, c), 10))
        assertArrayEquals(intArrayOf(1), taken.removed); assertEquals(0, taken.added.size); assertFalse(taken.appended(3, 2))
        val first = requireNotNull(objectsDiff(listOf(b, c), listOf(a, b, c), 10))
        assertArrayEquals(intArrayOf(0), first.added); assertFalse(first.appended(2, 3))
        // Moved to the front: only the object that moved, at both places.
        val moved = requireNotNull(objectsDiff(listOf(a, b, c, d), listOf(d, a, b, c), 10))
        assertArrayEquals(intArrayOf(3), moved.removed); assertArrayEquals(intArrayOf(0), moved.added)
        // Too many changed, or an object twice in one list.
        assertNull(objectsDiff(listOf(a, b, c, d), emptyList<RenderItem>(), 3))
        assertNull(objectsDiff(listOf(a), listOf(b, b, c), 10))
        assertNull(objectsDiff(listOf(a, b, c), listOf(c, b, b, a), 10))
    }
    @Test fun what_stays_keeps_its_order_and_is_as_much_as_can() {
        val random = java.util.Random(5)
        val pool = (0 until 40).map { item("o$it") }
        repeat(500) {
            val old = pool.filter { random.nextInt(3) != 0 }.shuffled(random).take(random.nextInt(30))
            val new = when (random.nextInt(4)) {
                // A few edits of the old list: removals, replacements, insertions, one move.
                0 -> old.toMutableList().apply {
                    repeat(random.nextInt(4)) { if (isNotEmpty()) removeAt(random.nextInt(size)) }
                    repeat(random.nextInt(3)) { if (isNotEmpty()) set(random.nextInt(size), item("n${random.nextInt(1000)}")) }
                    repeat(random.nextInt(3)) { add(random.nextInt(size + 1), item("i${random.nextInt(1000)}")) }
                    if (size > 1) { val at = random.nextInt(size); val moved = removeAt(at); add(random.nextInt(size + 1), moved) }
                }
                else -> pool.filter { random.nextInt(3) != 0 }.shuffled(random).take(random.nextInt(30))
            }
            val diff = requireNotNull(objectsDiff(old, new, 1_000))
            check(old, new, diff)
            // As few changed as there can be: what stays is a longest run common to both.
            val common = longestCommon(old, new)
            assertEquals(old.size - common, diff.removed.size)
            assertEquals(new.size - common, diff.added.size)
        }
    }
    /** The length of a longest common subsequence by identity (plain dynamic programming). */
    private fun <T : Any> longestCommon(a: List<T>, b: List<T>): Int {
        val table = Array(a.size + 1) { IntArray(b.size + 1) }
        for (i in a.indices.reversed()) for (j in b.indices.reversed())
            table[i][j] = if (a[i] === b[j]) table[i + 1][j + 1] + 1 else maxOf(table[i + 1][j], table[i][j + 1])
        return table[0][0]
    }
    @Test fun the_longest_run_is_found() {
        val run = longestIncreasing(intArrayOf(1, 0, 2, 3))
        assertEquals(3, run.count { it }); assertTrue(run[2] && run[3] && run[0] != run[1])
        assertEquals(3, longestIncreasing(intArrayOf(5, -1, 1, 2, -1, 3)).count { it })
        assertEquals(0, longestIncreasing(intArrayOf(-1, -1)).count { it })
        assertEquals(1, longestIncreasing(intArrayOf(9, 8, 7)).count { it })
    }
    // endregion

    // region The index of saved objects
    @Test fun the_index_finds_every_object_that_could_reach_an_area_in_list_order() {
        val random = java.util.Random(7)
        val count = 600
        val objects = List(count) { index -> item("o$index", bounds = when (index % 23) {
            0 -> Rect(Double.NaN, 0.0, 1.0, 1.0)
            1 -> Rect(-50.0, -30.0, 20.0, 10.0)            // off the page
            2 -> Rect(10.0, 10.0, 900.0, 700.0)            // over most of it
            3 -> Rect(400.0, 300.0, 0.0, 0.0)              // a dot
            else -> Rect(random.nextDouble() * 1100 - 50, random.nextDouble() * 900 - 50, random.nextDouble() * 60, random.nextDouble() * 60)
        }, shape = if (index % 17 == 0) Shape.Marker(1u, Point(0.0, 0.0), null) else Shape.Stroke("pen")) }
        fun index() = SavedIndex<RenderItem>(1000.0, 800.0, { it.bounds }, { boxHoldsDrawing(it) }, { it.objectId })
        val whole = index().apply { reset(objects) }
        var named = 0
        repeat(400) {
            val visible = Rect(random.nextDouble() * 1200 - 100, random.nextDouble() * 1000 - 100, random.nextDouble() * 120, random.nextDouble() * 120)
            val margin = random.nextDouble() * 5
            val found = requireNotNull(whole.near(visible, margin))
            val positions = found.map { objects.indexOf(it) }
            for (at in 1 until positions.size) assertTrue("in list order, once each", positions[at - 1] < positions[at])
            val set = found.toHashSet()
            for (o in objects) if (!boxHoldsDrawing(o) || boundsMayTouch(o.bounds, visible, margin))
                assertTrue("${o.objectId} reaches $visible but was not named", o in set)
            named += found.size
        }
        // A small area names a small part of the page, not every object.
        assertTrue("named on average ${named / 400} of $count", named / 400 < count / 4)
        // An area that cannot be judged: every object has to be looked at.
        assertNull(whole.near(Rect(Double.NaN, 0.0, 1.0, 1.0), 1.0))
        assertNull(whole.near(Rect(0.0, 0.0, 1.0, 1.0), Double.NaN))
        // Objects named besides what lies there, once each and in order.
        val far = objects[7]; val near = requireNotNull(whole.near(Rect(5_000.0, 5_000.0, 1.0, 1.0), 0.0, listOf(far, far)))
        assertEquals(1, near.count { it === far })
        // An empty page, and a page of no size.
        assertEquals(emptyList<RenderItem>(), index().apply { reset(emptyList()) }.near(Rect(0.0, 0.0, 10.0, 10.0), 1.0))
        val tiny = SavedIndex<RenderItem>(0.0, 0.0, { it.bounds }, { true }, { it.objectId }).apply { reset(listOf(item("t", bounds = Rect(5.0, 5.0, 1.0, 1.0)))) }
        assertEquals(1, requireNotNull(tiny.near(Rect(0.0, 0.0, 100.0, 100.0), 0.0)).size)
        assertEquals("o5", whole.find("o5")?.objectId); assertNull(whole.find("nobody"))
    }
    /** After each change the index follows by what changed, and answers as an
     * index made from nothing for the new list would. */
    @Test fun the_index_follows_its_list_by_what_changed() {
        val random = java.util.Random(13)
        var serial = 0
        fun fresh() = item("s${serial++}", bounds = Rect(random.nextDouble() * 1000, random.nextDouble() * 800, random.nextDouble() * 50, random.nextDouble() * 50))
        var list = List(300) { fresh() }
        val followed = SavedIndex<RenderItem>(1000.0, 800.0, { it.bounds }, { true }, { it.objectId }).apply { reset(list) }
        repeat(400) { round ->
            val next = list.toMutableList()
            when (round % 5) {
                0 -> repeat(1 + random.nextInt(3)) { next.add(fresh()) }                                  // strokes saved
                1 -> if (next.isNotEmpty()) next.removeAt(random.nextInt(next.size))                       // undo, erase, delete
                2 -> if (next.isNotEmpty()) next[random.nextInt(next.size)] = fresh()                       // edit, move, computer
                3 -> if (next.size > 1) { val moved = next.removeAt(random.nextInt(next.size)); next.add(random.nextInt(next.size + 1), moved) }
                else -> repeat(1 + random.nextInt(4)) { next.add(random.nextInt(next.size + 1), fresh()) } // put in anywhere, many at one place
            }
            followed.follow(next, requireNotNull(objectsDiff(list, next, 1_024)))
            list = next
            assertEquals(list.size, followed.size)
            // The keys grow along the list.
            for (at in 1 until list.size) assertTrue(requireNotNull(followed.keyOf(list[at - 1])) < requireNotNull(followed.keyOf(list[at])))
            val made = SavedIndex<RenderItem>(1000.0, 800.0, { it.bounds }, { true }, { it.objectId }).apply { reset(list) }
            repeat(5) {
                val visible = Rect(random.nextDouble() * 1000, random.nextDouble() * 800, random.nextDouble() * 150, random.nextDouble() * 150)
                val got = requireNotNull(followed.near(visible, 2.0)); val want = requireNotNull(made.near(visible, 2.0))
                assertEquals(want.size, got.size)
                for (at in want.indices) assertSame(want[at], got[at])
            }
            for (o in list) assertSame(o, followed.find(o.objectId))
        }
        // Hundreds of objects put in at one place run the keys out of room once in a while; they are given out again.
        val crowded = List(4) { fresh() }
        val dense = SavedIndex<RenderItem>(1000.0, 800.0, { it.bounds }, { true }, { it.objectId }).apply { reset(crowded) }
        var held = crowded
        repeat(120) {
            // Between the second object (key 1) and the one put in last: each key halves the room left.
            val next = held.toMutableList().apply { add(2, fresh()) }
            dense.follow(next, requireNotNull(objectsDiff(held, next, 1_024))); held = next
        }
        for (at in 1 until held.size) assertTrue(requireNotNull(dense.keyOf(held[at - 1])) < requireNotNull(dense.keyOf(held[at])))
        assertTrue(dense.renumbered > 0)
    }
    /** Runs of objects put in together into a gap only a few steps of a
     * double wide: every key is still above the one before it, so that what is
     * found comes in list order and each object once. */
    @Test fun objects_put_in_together_into_a_tiny_gap_still_get_keys_in_order() {
        var serial = 0
        // All on one spot: every area finds all of them.
        fun fresh() = item("t${serial++}", bounds = Rect(10.0, 10.0, 5.0, 5.0))
        var held = List(3) { fresh() }
        val index = SavedIndex<RenderItem>(100.0, 100.0, { it.bounds }, { true }, { it.objectId }).apply { reset(held) }
        repeat(240) { round ->
            // Into the same place each time, right after the second object:
            // one at a time (the room halves), and every third time six at once.
            val next = held.toMutableList().apply { repeat(if (round % 3 == 2) 6 else 1) { add(2 + it, fresh()) } }
            index.follow(next, requireNotNull(objectsDiff(held, next, 1_024)))
            held = next
            for (at in 1 until held.size) assertTrue("round $round at $at", requireNotNull(index.keyOf(held[at - 1])) < requireNotNull(index.keyOf(held[at])))
            val found = requireNotNull(index.near(Rect(0.0, 0.0, 100.0, 100.0), 0.0))
            assertEquals(held.size, found.size)
            for (at in held.indices) assertSame(held[at], found[at])
        }
        assertTrue(index.renumbered > 0)
    }
    // endregion

    // region What a gesture changes
    private val layers = listOf("notes", "top")
    private fun basis(objects: List<RenderItem>, hidden: Set<String> = emptySet()) =
        PictureBasis(1000, 1000, Transform(), 1.0, null, true, layers, hidden, objects)
    private fun area(held: List<RenderItem>, next: List<RenderItem>, transformed: Map<String, Transform> = emptyMap(),
                     heldHidden: Set<String> = emptySet(), hidden: Set<String> = emptySet()): TouchedArea? =
        touchedArea(basis(held, heldHidden), DryLayers(layers, held, heldHidden), basis(next, hidden), transformed,
            requireNotNull(objectsDiff(held, next, 1_024)), { id -> next.firstOrNull { it.objectId == id } }) { it }

    @Test fun a_stroke_being_moved_touches_where_it_was_and_where_it_is() {
        val stroke = item("s", bounds = Rect(100.0, 100.0, 20.0, 10.0))
        val other = item("o", bounds = Rect(600.0, 600.0, 10.0, 10.0))
        val moved = requireNotNull(area(listOf(stroke, other), listOf(stroke, other), mapOf("s" to Transform(e = 300.0))))
        assertEquals(setOf(PixelBox(98, 98, 122, 112), PixelBox(398, 98, 422, 112)), moved.boxes.toSet())
        // Not moved yet: nothing touched.
        assertTrue(requireNotNull(area(listOf(stroke), listOf(stroke), mapOf("s" to Transform()))).isEmpty)
    }
    @Test fun objects_hidden_while_erased_or_shown_again_touch_where_they_lie() {
        val stroke = item("s", bounds = Rect(100.0, 100.0, 20.0, 10.0))
        val hiddenNow = requireNotNull(area(listOf(stroke), listOf(stroke), hidden = setOf("s")))
        assertEquals(listOf(PixelBox(98, 98, 122, 112)), hiddenNow.boxes)
        val back = requireNotNull(area(listOf(stroke), listOf(stroke), heldHidden = setOf("s")))
        assertEquals(listOf(PixelBox(98, 98, 122, 112)), back.boxes)
    }
    @Test fun what_cannot_be_drawn_by_parts_is_not_placed() {
        val a = item("a"); val marker = item("m", shape = Shape.Marker(1u, Point(0.0, 0.0), null))
        // A marker being moved.
        assertNull(area(listOf(a, marker), listOf(a, marker), mapOf("m" to Transform(e = 5.0))))
        // A stroke shown in a layer the document does not list, or composited otherwise than its layer.
        assertNull(area(listOf(a), listOf(a, item("u", layer = "unlisted"))))
        assertNull(area(listOf(a), listOf(a, item("t", opacity = 0.5))))
        // A layer that drew nothing takes the first object shown in it; a second one composited otherwise does not fit.
        assertNotNull(area(listOf(a), listOf(a, item("h", layer = "top", blend = "multiply"))))
        assertNull(area(listOf(a), listOf(a, item("h", layer = "top", blend = "multiply"), item("p", layer = "top"))))
        // A hidden object shown again composited otherwise than its layer.
        val faint = item("t", opacity = 0.5)
        assertNull(area(listOf(a, faint), listOf(a, faint), heldHidden = setOf("t")))
        val plain = item("p")
        assertNotNull(area(listOf(a, plain), listOf(a, plain), heldHidden = setOf("p")))
    }
    // endregion
}
