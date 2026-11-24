package com.visualworkbench.android.editor

import android.os.SystemClock
import android.util.Log
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicLongArray
import java.util.concurrent.locks.ReentrantLock

/** Lifecycle diagnostics of the canvas surface, for telling a slow device from a
 * redraw the canvas lost. One log tag; fixed words and numbers only (ordinal
 * numbers of surfaces, generations, frames and scene revisions, pixel sizes,
 * milliseconds, state names), never content, paths or identifiers.
 *
 * Off unless switched on for this process before the canvas is created: on a
 * device with `adb shell setprop log.tag.VW_CANVAS_LIFE VERBOSE` (the setting
 * lasts until the device restarts), or by an on-screen test through [forced].
 * While off, nothing is recorded, kept or started.
 *
 * Strictly observational while on: the thread that sees an event (Main, a
 * drawing thread, a commit callback) reads the clock once and writes a few
 * numbers into a preallocated ring ([CanvasLifeRing]). It formats nothing and
 * writes no log. The log's own low-priority thread ([CanvasLifeRecorder]) turns
 * the records into lines while a canvas made with the log on is open, and the
 * on-screen test drains them before it writes or reads lines itself. No
 * transaction, listener or callback is added to the window, the surface or a
 * frame for the log's sake. */
internal object CanvasLife {
    const val TAG = "VW_CANVAS_LIFE"
    /** On-screen tests only. */
    @Volatile var forced = false
    private val surfaces = AtomicInteger()
    private val history = CanvasLifeHistory(240)
    /** Made with the first surface that has the log on; null until then, so a
     * process that never switches the log on allocates and starts nothing for it. */
    @Volatile private var recorder: CanvasLifeRecorder? = null
    private const val RING_RECORDS = 1_024
    private const val WRITE_PERIOD_MS = 100L
    /** The writer stops one second after the last canvas left and nothing more arrived. */
    private const val LINGER_ROUNDS = 10
    /** The longest an on-screen test waits for the writer before it writes its own line. */
    private const val LINE_WAIT_MS = 50L

    fun enabled(): Boolean = forced || Log.isLoggable(TAG, Log.VERBOSE)

    /** Main, for a surface made while the log is on: its number. */
    fun nextSurface(): Int {
        if (recorder == null) synchronized(this) {
            if (recorder == null) recorder = CanvasLifeRecorder(RING_RECORDS, SystemClock::uptimeMillis, ::emit, ::startWriter,
                ::pause, LINGER_ROUNDS, LINE_WAIT_MS)
        }
        return surfaces.incrementAndGet()
    }

    /** Main: a canvas made while the log is on was attached; the writer runs from now. */
    fun opened() { recorder?.opened() }

    /** Main: that canvas has left its window and its retirement is confirmed. */
    fun closed() { recorder?.closed() }

    /** Any thread, for a surface made while the log is on: one record. The caller
     * has read the clock; this writes the numbers into the ring and returns. */
    fun record(event: CanvasLifeEvent, surface: Int, timeMs: Long, a: Long, b: Long, c: Long, d: Long, e: Long, f: Long,
               g: Long, h: Long, i: Long, j: Long, k: Long, l: Long) {
        recorder?.ring?.write(event.ordinal, surface, timeMs, a, b, c, d, e, f, g, h, i, j, k, l)
    }

    /** On-screen tests. Writes what the canvases recorded so far, then [text]. `t`
     * is the device's uptime clock in milliseconds. */
    fun line(text: String) {
        val current = recorder
        if (current == null) emit("t=${SystemClock.uptimeMillis()} $text") else current.line(text)
    }

    /** On-screen tests: the newest lines in the order of their `t`, including the records drained now. */
    fun recent(): List<String> {
        recorder?.drain()
        return history.snapshot().sortedBy { it.substringAfter("t=").substringBefore(' ').toLongOrNull() ?: 0L }
    }

    private fun emit(line: String) {
        history.add(line)
        Log.i(TAG, line)
    }

    private fun startWriter(task: Runnable) {
        Thread(task, "VwCanvasLife").apply { isDaemon = true; priority = Thread.MIN_PRIORITY; start() }
    }

    private fun pause(): Boolean = try { Thread.sleep(WRITE_PERIOD_MS); true } catch (_: InterruptedException) { false }
}

/** The lifecycle log's writer, once a canvas had the log on. It runs while at least
 * one such canvas is open (from its attach until its retirement after detach is
 * confirmed), and afterwards until every record claimed is written and
 * [lingerRounds] rounds passed with nothing new, so a retired canvas's last
 * records are written too; the next canvas starts it again. Each round drains the
 * ring in batches of [WRITER_BATCH], so an on-screen test that wants to write a
 * line waits at most one batch, and never longer than [lineWaitMs].
 *
 * [startWriter] starts a thread that runs the task given; [pause] waits one round
 * and returns false when the thread was interrupted. */
internal class CanvasLifeRecorder(
    capacity: Int,
    private val clock: () -> Long,
    private val emit: (String) -> Unit,
    private val startWriter: (Runnable) -> Unit,
    private val pause: () -> Boolean,
    private val lingerRounds: Int,
    private val lineWaitMs: Long,
) {
    val ring = CanvasLifeRing(capacity)
    /** Orders drained records and the test's own lines. Main never takes it. */
    private val drainLock = ReentrantLock()
    private val sink = CanvasLifeRing.Sink { kind, surface, timeMs, values -> emit(formatCanvasLifeRecord(kind, surface, timeMs, values)) }
    // Under this recorder's monitor, which Main takes only to open or close a canvas.
    private var open = 0
    private var writing = false
    private var quietRounds = 0
    init { require(lingerRounds > 0 && lineWaitMs >= 0) }

    /** Whether the writer runs now. */
    val writerRunning: Boolean get() = synchronized(this) { writing }

    /** Main: a canvas opened; starts the writer when none runs. */
    fun opened() {
        val start = synchronized(this) {
            open++; quietRounds = 0
            if (writing) false else { writing = true; true }
        }
        if (start) startWriter(Runnable { while (writeRound()) if (!pause()) { synchronized(this) { writing = false }; return@Runnable } })
    }

    /** Main: a canvas that opened has closed. */
    fun closed() { synchronized(this) { if (open > 0) open-- } }

    /** The writer: one round. Writes every record ready, then tells whether to go on;
     * false once no canvas is open, nothing claimed is left and the round was the
     * last of [lingerRounds] quiet ones. */
    fun writeRound(): Boolean {
        val delivered = drainInBatches()
        // Asked outside this monitor, so Main never waits behind a drain for it.
        val left = ring.undelivered()
        synchronized(this) {
            quietRounds = if (open > 0 || delivered > 0 || left) 0 else quietRounds + 1
            if (quietRounds < lingerRounds) return true
            writing = false
            return false
        }
    }

    /** On-screen tests: writes the records ready, then [text]. */
    fun line(text: String) {
        val locked = lockBriefly()
        try { if (locked) drainLocked(Int.MAX_VALUE); emit("t=${clock()} $text") }
        finally { if (locked) drainLock.unlock() }
    }

    /** On-screen tests: writes the records ready. */
    fun drain() {
        if (!lockBriefly()) return
        try { drainLocked(Int.MAX_VALUE) } finally { drainLock.unlock() }
    }

    private fun drainInBatches(): Int {
        var total = 0
        while (true) {
            drainLock.lock()
            val delivered = try { drainLocked(WRITER_BATCH) } finally { drainLock.unlock() }
            total += delivered
            if (delivered < WRITER_BATCH) return total
        }
    }

    /** Under [drainLock]: records delivered now, then a line for any lost since the last one. */
    private fun drainLocked(max: Int): Int {
        val delivered = ring.drain(max, sink)
        val lost = ring.takeDropped()
        if (lost > 0) emit("t=${clock()} records_dropped count=$lost")
        return delivered
    }

    private fun lockBriefly(): Boolean =
        try { drainLock.tryLock(lineWaitMs, TimeUnit.MILLISECONDS) } catch (_: InterruptedException) { Thread.currentThread().interrupt(); false }

    companion object {
        const val WRITER_BATCH = 32
    }
}

/** The records of the canvas lifecycle log. Each template is the line written for
 * a record: `#` is the next number, `?` the next number as a flag, `%` the next two
 * numbers as a size (`none` when negative), and `{a|b|c}` the word at the next
 * number's position (the number itself when there is no such word). */
internal enum class CanvasLifeEvent(private val template: String) {
    SurfaceCreated("surface_created view=%"),
    SurfaceChanged("surface_changed size=% changed=?"),
    SurfaceDestroyed("surface_destroyed"),
    RedrawRequest("redraw_request presented=? held=# current=# kept=#"),
    RedrawFallback("redraw_fallback held=# current=# current_presented=?"),
    RedrawFinish("redraw_finish cause=${lifeWords(CanvasLifeWords.redrawCauses)} released=#"),
    Attach("attach"),
    Detach("detach current=# kept=# releases=#"),
    RetirementConfirmed("retirement_confirmed after_detach_ms=#"),
    FrameRequest("frame_request gen=# cause=${lifeWords(CanvasLifeWords.frameCauses)} rev=# scene_view=% " +
        "result=${lifeWords(CanvasLifeWords.frameResults)} in_flight=#"),
    Quiet("quiet gen=# frames=#/# drawn_rev=# newest_rev=# in_flight=# quiet_requests=# quiet_granted=# " +
        "quiet_frames=# ink_frames=#"),
    Resize("resize action=${lifeWords(CanvasReplacement.ResizeAction.entries.map { it.name })} size=% current=# " +
        "current_presented=? kept=# releases=#"),
    ResizeWait("resize_wait gen=# kept=# waited_ms=# presented=?"),
    KeptFallback("kept_fallback gen=# retired=? resize_redraw_pending=?"),
    KeptRedrawn("kept_redrawn gen=# retires=?"),
    KeptRestored("kept_restored gen=#"),
    ReplaceAfterSettled("replace_after_settled kept=#"),
    GenerationCreated("generation_created gen=# size=% view=% rev=# scene_view=% kept=# scenes_without_generation=#"),
    Presented("presented gen=# after_created_ms=#"),
    PresentedNotCurrent("presented_not_current gen=# current=#"),
    SuccessorPresented("successor_presented gen=# kept=# kept_retires=?"),
    Retire("retire reason=${lifeWords(CanvasRetirementReason.entries.map { it.name })} current=# kept=#"),
    Release("release gen=# reason=${lifeWords(CanvasRetirementReason.entries.map { it.name })} presented=? " +
        "in_flight=# request_pending=? resize_redraw_pending=? started=# completed=#"),
    ReleaseCallback("release_callback gen=# after_ms=#"),
    ReleaseSettled("release_settled gen=# after_ms=# reader_closed=? releases=#"),
    GenerationWaits("generation_waits reason=${lifeWords(CanvasLifeWords.waitReasons)} releases=# " +
        "awaited_retirement=${lifeWords(listOf("None") + CanvasRetirementReason.entries.map { it.name })} " +
        "awaited_milestone=${lifeWords(listOf("None") + CanvasRetirementMilestone.entries.map { it.name })}"),
    SceneWithoutGeneration("scene_without_generation rev=# scene_view=%"),
    Draw("draw gen=# frame=# rev=# scene_view=% buffer=% requests=# in_flight=# newest_rev=# ink_frames=# closed=?"),
    FrameReady("frame_ready gen=# frame=# started=# visible=? new_layer=? resize_redraw=? in_flight=# " +
        "request_pending=? drawn_rev=# newest_rev=#");

    /** How many numbers a record of this event carries. */
    val arity: Int = run {
        var count = 0; var at = 0
        while (at < template.length) {
            when (template[at]) {
                '#', '?' -> count++
                '%' -> count += 2
                '{' -> { count++; at = template.indexOf('}', at).also { require(it > 0) { "Unclosed word list in $name" } } }
            }
            at++
        }
        require(count <= CanvasLifeRing.VALUES) { "$name carries $count numbers" }
        count
    }

    /** The template's first word: what the line says happened. */
    val word: String get() = template.substringBefore(' ')

    /** Appends this record's line, without its time and surface, to [out]. */
    fun render(values: LongArray, out: StringBuilder) {
        var next = 0; var at = 0
        while (at < template.length) {
            when (val c = template[at]) {
                '#' -> out.append(values[next++])
                '?' -> out.append(values[next++] != 0L)
                '%' -> {
                    val width = values[next++]; val height = values[next++]
                    if (width < 0 || height < 0) out.append("none") else out.append(width).append('x').append(height)
                }
                '{' -> {
                    val end = template.indexOf('}', at)
                    val words = template.substring(at + 1, end).split('|')
                    val value = values[next++]
                    if (value >= 0 && value < words.size) out.append(words[value.toInt()]) else out.append(value)
                    at = end
                }
                else -> out.append(c)
            }
            at++
        }
    }
}

/** The fixed words some canvas lifecycle records carry, by position in each list. */
internal object CanvasLifeWords {
    val redrawCauses = listOf("surface_destroyed", "detached", "presented")
    val frameCauses = listOf("publish", "created", "newer_scene", "settled")
    val frameResults = listOf("granted", "closed", "not_current", "request_pending", "draw_in_flight")
    val waitReasons = listOf("detached", "no_scene", "no_surface", "viewport_changing", "release_settling", "controller_gate", "other")
}

private fun lifeWords(words: List<String>): String = words.joinToString("|", "{", "}")

/** One record's line: `t` (uptime in milliseconds when it was recorded), the
 * surface's number, then the event's own words and numbers. */
internal fun formatCanvasLifeRecord(kind: Int, surface: Int, timeMs: Long, values: LongArray): String {
    val out = StringBuilder(128).append("t=").append(timeMs).append(" surface=").append(surface).append(' ')
    val event = CanvasLifeEvent.entries.getOrNull(kind)
    if (event == null) out.append("unknown_record kind=").append(kind) else event.render(values, out)
    return out.toString()
}

/** Lifecycle diagnostics only: a fixed ring of numeric records. Any thread writes
 * one without locks, allocation or formatting: it claims the next record number,
 * marks the slot as being written, stores the numbers with a check value and
 * publishes the slot. One reader at a time drains the records in the order they
 * were claimed. A record its writer is still storing holds back the ones after it
 * until the next drain. A record overwritten before it was drained (the ring was
 * full), mixed with another, or missing from its slot when due (its writer stalled
 * while the ring wrapped round) is counted as dropped, never delivered. */
internal class CanvasLifeRing(private val capacity: Int) {
    fun interface Sink {
        /** [values] is only valid during the call. */
        fun record(kind: Int, surface: Int, timeMs: Long, values: LongArray)
    }

    private val slots = AtomicLongArray(capacity * STRIDE)
    /** Per slot: the number of its complete record plus one, or minus that while it is written. */
    private val marks = AtomicLongArray(capacity)
    private val claimed = AtomicLong()
    // The reader's own state, under this ring's monitor.
    private var next = 0L
    private var dropped = 0L
    private val read = LongArray(STRIDE)
    private val values = LongArray(VALUES)
    init { require(capacity > 0) }

    fun write(kind: Int, surface: Int, timeMs: Long, a: Long, b: Long, c: Long, d: Long, e: Long, f: Long,
              g: Long, h: Long, i: Long, j: Long, k: Long, l: Long) =
        put(claim(), kind, surface, timeMs, a, b, c, d, e, f, g, h, i, j, k, l)

    /** A writer's first step: the next record number. Tests call it apart from [put]
     * to stand for a writer stalled between the two steps. */
    fun claim(): Long = claimed.getAndIncrement()

    /** A writer's second step: stores record [number] in its slot and publishes it. */
    fun put(number: Long, kind: Int, surface: Int, timeMs: Long, a: Long, b: Long, c: Long, d: Long, e: Long, f: Long,
            g: Long, h: Long, i: Long, j: Long, k: Long, l: Long) {
        val slot = (number % capacity).toInt()
        val base = slot * STRIDE
        marks.set(slot, -(number + 1))
        slots.lazySet(base, kind.toLong()); slots.lazySet(base + 1, surface.toLong()); slots.lazySet(base + 2, timeMs)
        slots.lazySet(base + 3, a); slots.lazySet(base + 4, b); slots.lazySet(base + 5, c); slots.lazySet(base + 6, d)
        slots.lazySet(base + 7, e); slots.lazySet(base + 8, f); slots.lazySet(base + 9, g); slots.lazySet(base + 10, h)
        slots.lazySet(base + 11, i); slots.lazySet(base + 12, j); slots.lazySet(base + 13, k); slots.lazySet(base + 14, l)
        var check = mix(mix(mix(number + 1, kind.toLong()), surface.toLong()), timeMs)
        check = mix(mix(mix(mix(mix(mix(check, a), b), c), d), e), f)
        check = mix(mix(mix(mix(mix(mix(check, g), h), i), j), k), l)
        slots.lazySet(base + CHECK, check)
        marks.set(slot, number + 1)
    }

    /** Delivers the complete records not delivered yet, oldest first and at most
     * [max] of them; returns how many. Only a record its own writer is still
     * storing holds back the ones after it. A slot that holds anything else than
     * the record due (a later record, an older one whose writer stalled while the
     * ring wrapped round, or nothing yet) counts the record due as dropped. */
    @Synchronized fun drain(max: Int = Int.MAX_VALUE, sink: Sink): Int {
        val end = claimed.get()
        if (end - next > capacity) { dropped += end - next - capacity; next = end - capacity }
        var delivered = 0
        while (next < end && delivered < max) {
            val slot = (next % capacity).toInt()
            val complete = next + 1
            val mark = marks.get(slot)
            if (mark == complete) {
                val base = slot * STRIDE
                for (at in 0 until STRIDE) read[at] = slots.get(base + at)
                var check = complete
                for (at in 0 until CHECK) check = mix(check, read[at])
                if (marks.get(slot) == complete && check == read[CHECK]) {
                    read.copyInto(values, 0, 3, CHECK)
                    sink.record(read[0].toInt(), read[1].toInt(), read[2], values)
                    delivered++
                } else dropped++ // Overwritten while it was read, or mixed with another record.
            } else if (mark == -complete) break // Its writer is storing it: it and the ones after wait for the next drain.
            else dropped++
            next++
        }
        return delivered
    }

    /** Whether a record has been claimed that was neither delivered nor dropped yet. */
    @Synchronized fun undelivered(): Boolean = claimed.get() > next

    /** Records lost since the last call, then zero. */
    @Synchronized fun takeDropped(): Long = dropped.also { dropped = 0 }

    private fun mix(check: Long, value: Long): Long = check * MIX + value

    companion object {
        /** Numbers one record carries. */
        const val VALUES = 12
        /** Slot layout: kind, surface, time, the numbers, then the check value. */
        private const val CHECK = 3 + VALUES
        private const val STRIDE = CHECK + 1
        /** An odd 64-bit multiplier (the golden ratio's), so every stored number moves the check value. */
        private const val MIX = -7046029254386353131L
    }
}

/** The newest lifecycle lines of the canvas, kept in memory only while that
 * diagnostic is switched on, so an on-screen test can put them in its failure
 * message after the device log has rolled over. */
internal class CanvasLifeHistory(private val capacity: Int) {
    private val lines = ArrayDeque<String>()
    init { require(capacity > 0) }

    fun add(line: String) {
        synchronized(lines) { if (lines.size == capacity) lines.removeFirst(); lines.addLast(line) }
    }

    fun snapshot(): List<String> = synchronized(lines) { lines.toList() }
}

/** Lifecycle diagnostics only: which per-frame records of one canvas generation are
 * made. Hovering, panning and zooming publish a saved-frame scene per input
 * sample, so recording each would flood the log and the in-memory history. After
 * each lifecycle event ([again]) the next [frames] saved frames and [requests]
 * frame requests are recorded in full; beyond that they are only counted, and at
 * most one summary per [summaryMs] reports the counts. Frames are numbered from 1
 * in the order they start and, separately, in the order they complete; both
 * orders agree, so a frame whose start was recorded also has its completion
 * recorded. The caller holds the generation's lock. Nothing here allocates: a
 * summary's counts are left in the `taken` fields for the caller to record. */
internal class CanvasLifeBudget(private val frames: Int, private val requests: Int, private val summaryMs: Long) {
    private var framesUntil = frames
    private var requestsLeft = requests
    private var quietRequests = 0
    private var quietGranted = 0
    private var quietFrames = 0
    private var inkFrames = 0
    private var summarized = false
    private var lastSummaryMs = 0L
    /** The counts of the summary last taken (after [again] or [summaryDue] returned true). */
    var takenRequests = 0; private set
    var takenGranted = 0; private set
    var takenFrames = 0; private set
    var takenInk = 0; private set
    init { require(frames >= 0 && requests >= 0 && summaryMs > 0) }

    /** A lifecycle event: frames after [started] and the next requests are recorded
     * again. True when the counts gathered since the last summary were taken. */
    fun again(started: Int, nowMs: Long): Boolean {
        framesUntil = maxOf(framesUntil, started + frames); requestsLeft = requests
        return takeSummary(nowMs)
    }

    /** Whether the frame numbered [number] (by start or by completion) is recorded. */
    fun frameWritten(number: Int): Boolean = number <= framesUntil

    /** A frame request; true when it is recorded, false when it is only counted. */
    fun request(granted: Boolean): Boolean {
        if (requestsLeft > 0) { requestsLeft--; return true }
        quietRequests++; if (granted) quietGranted++
        return false
    }

    /** A saved frame completed without being recorded. */
    fun quietFrame() { quietFrames++ }

    /** An ink frame was drawn; ink frames are only ever counted. */
    fun inkFrame() { inkFrames++ }

    /** Ink frames since the last count taken, then zero. */
    fun takeInk(): Int = inkFrames.also { inkFrames = 0 }

    /** True when at least [summaryMs] has passed since the last summary, something
     * was counted, and the counts were taken. */
    fun summaryDue(nowMs: Long): Boolean {
        if (summarized && nowMs - lastSummaryMs < summaryMs) return false
        return takeSummary(nowMs)
    }

    private fun takeSummary(nowMs: Long): Boolean {
        if (quietRequests == 0 && quietFrames == 0 && inkFrames == 0) return false
        takenRequests = quietRequests; takenGranted = quietGranted; takenFrames = quietFrames; takenInk = takeInk()
        quietRequests = 0; quietGranted = 0; quietFrames = 0; summarized = true; lastSummaryMs = nowMs
        return true
    }
}
