package com.visualworkbench.shared

/** Main-thread confined Air Actions admission in front of the existing serial
 * dispatcher. It owns no editor, native input, timer, permission or session.
 * Host calls advance at nextDeadlineMs, and interrupt on every admission loss.
 * Delivery rechecks the original ticket and the host's full existing guard. */
public class AirActionRouter(
    private val dispatcher: CommandDispatcher,
    private val nowMs: () -> Long,
    private val binding: () -> AirActionBinding?,
    private val allowed: (AirActionBinding, AirActionCommand) -> Boolean,
    private val execute: suspend (AirActionBinding, AirActionCommand) -> Unit,
    private val queueRefused: () -> Unit = {},
) {
    private data class Key(val stream: Int, val gesture: AirActionGesture, val down: Long)
    private data class Press(val binding: AirActionBinding, val generation: Long, val coalesced: Boolean, val observedAt: Long)
    private data class Pending(val gesture: AirActionGesture, val command: AirActionCommand,
        val binding: AirActionBinding, val generation: Long, val time: Long, val stream: Int)

    private var bindings = AirActionBindings()
    private var enabled = false
    private var closed = false
    private var generation = 0L
    private var lastClock = -1L
    private var barrier = -1L
    private val pressed = mutableMapOf<Key, Press>()
    private val seen = mutableMapOf<Pair<Int, AirActionGesture>, Long>()
    private val pending = mutableListOf<Pending>()
    private var barrelDown = false
    private var barrelUntil = -1L
    private val clickResolved = mutableMapOf<Int, Pair<AirActionBinding, Long>>()

    public val nextDeadlineMs: Long? get() = pending.minOfOrNull { it.time + ResolutionMs }

    /** A remap or availability change retires all old presses and queued offers. */
    public fun configure(bindings: AirActionBindings, enabled: Boolean): Unit {
        if (closed || (this.bindings == bindings && this.enabled == enabled)) return
        interrupt()
        this.bindings = bindings
        this.enabled = enabled
    }

    /** True means a declared key was consumed, including repeats/refusals. A
     * command requires a balanced, uncanceled pair in the same admitted session. */
    public fun key(event: AirActionKeyEvent): Boolean {
        val now = clock() ?: return true
        if (closed || !enabled || !valid(event, now)) return true
        val key = Key(event.stream, event.gesture, event.downTimeMs)
        when (event.phase) {
            AirActionKeyPhase.Multiple -> return true
            AirActionKeyPhase.Down -> {
                if (event.canceled) { pressed.remove(key); remember(key); return true }
                if (event.repeatCount != 0 || key.down <= (seen[key.stream to key.gesture] ?: -1L) || key in pressed) return true
                if (key.stream to key.gesture !in seen && seen.size >= RememberedStreams) { queueRefused(); return true }
                remember(key)
                val ticket = binding() ?: return true
                if (ticket.context == AirActionContext.Dialog) return true
                val single = pending.lastOrNull { it.gesture == AirActionGesture.SingleClick &&
                    it.stream == event.stream && it.binding == ticket && event.eventTimeMs >= it.time &&
                    event.eventTimeMs - it.time <= ResolutionMs &&
                    event.gesture in setOf(AirActionGesture.SingleClick, AirActionGesture.DoubleClick) }
                if (single != null) pending.remove(single)
                val command = bindings.command(event.gesture) ?: return true
                if (!allowed(ticket, command) || pressed.size >= MaximumPressed) return true
                pressed[key] = Press(ticket, generation, single != null, event.eventTimeMs)
            }
            AirActionKeyPhase.Up -> {
                val press = pressed.remove(key) ?: return true
                remember(key)
                if (event.canceled || event.repeatCount != 0 || press.generation != generation ||
                    press.binding != binding() || event.eventTimeMs < press.observedAt) return true
                complete(event.gesture, event.eventTimeMs, press, event.stream)
            }
        }
        return true
    }

    /** Observe MotionEvent.BUTTON_STYLUS_PRIMARY before its existing barrel
     * handler. That handler retains hold/peek, contact queuing and click authority.
     * Air keys within the correlation window lose to the physical barrel route. */
    public fun barrel(active: Boolean, eventTimeMs: Long): Unit {
        val now = clock() ?: return
        if (closed || eventTimeMs < 0 || eventTimeMs > now || now - eventTimeMs > MaximumPressMs) return
        if (active) {
            barrelDown = true
            pending.clear()
            // Retire already offered Air commands before the barrel handler runs.
            generation++
            pressed.clear()
        } else if (barrelDown) {
            barrelDown = false
            barrelUntil = eventTimeMs + ResolutionMs
        }
    }

    public fun advance(): Unit {
        val now = clock() ?: return
        val ready = pending.filter { now >= it.time + ResolutionMs }
        pending.removeAll(ready.toSet())
        for (candidate in ready) {
            if (!admitted(candidate, now)) continue
            val capturedGeneration = generation
            val accepted = dispatcher.offer {
                val deliveredAt = nowMs()
                if (!closed && enabled && generation == capturedGeneration &&
                    candidate.binding == binding() && deliveredAt >= candidate.time &&
                    deliveredAt - candidate.time <= MaximumPressMs && allowed(candidate.binding, candidate.command)) {
                    execute(candidate.binding, candidate.command)
                }
            }
            if (!accepted) queueRefused() // Exact accepted command is never retried.
        }
    }

    /** Also invalidates offers already waiting in CommandDispatcher. Invalidate
     * before closing the parent project or resetting a native grant. */
    public fun interrupt(): Unit {
        generation++
        pressed.clear()
        pending.clear()
        barrelDown = false
        barrelUntil = -1L
        clickResolved.clear()
        barrier = maxOf(barrier, lastClock, nowMs())
        seen.clear()
    }

    public fun close(): Unit { if (!closed) { interrupt(); closed = true; enabled = false } }

    private fun clock(): Long? {
        val now = nowMs()
        if (now < 0 || now < lastClock || now > Long.MAX_VALUE - MaximumPressMs) {
            interrupt()
            return null
        }
        lastClock = now
        pressed.keys.filter { now - it.down > MaximumPressMs }.forEach { pressed.remove(it); remember(it) }
        return now
    }

    private fun valid(event: AirActionKeyEvent, now: Long): Boolean = event.downTimeMs > barrier &&
        event.downTimeMs >= 0 && event.eventTimeMs >= event.downTimeMs && event.eventTimeMs <= now &&
        now - event.downTimeMs <= MaximumPressMs && event.repeatCount >= 0

    private fun complete(gesture: AirActionGesture, time: Long, press: Press, stream: Int) {
        if (barrelDown || time <= barrelUntil) return
        var selected = gesture
        if (gesture == AirActionGesture.SingleClick || gesture == AirActionGesture.DoubleClick) {
            val resolved = clickResolved[stream]
            if (resolved?.first == press.binding && time >= resolved.second && time - resolved.second <= ResolutionMs) return
            if (gesture == AirActionGesture.DoubleClick || press.coalesced) {
                selected = AirActionGesture.DoubleClick
                clickResolved[stream] = press.binding to time
            }
        }
        val command = bindings.command(selected) ?: return
        if (!allowed(press.binding, command)) return
        if (pending.size >= MaximumPending) { queueRefused(); return }
        pending += Pending(selected, command, press.binding, generation, time, stream)
    }

    private fun admitted(candidate: Pending, now: Long): Boolean = !closed && enabled &&
        candidate.generation == generation && candidate.binding == binding() &&
        candidate.binding.context != AirActionContext.Dialog && !barrelDown &&
        now > barrelUntil && now - candidate.time <= MaximumPressMs && allowed(candidate.binding, candidate.command)

    private fun remember(key: Key) {
        val stream = key.stream to key.gesture
        if (stream in seen || seen.size < RememberedStreams) seen[stream] = maxOf(key.down, seen[stream] ?: -1L)
    }

    public companion object {
        public const val ResolutionMs: Long = 250L
        public const val MaximumPressMs: Long = 2_000L
        private const val MaximumPressed = 8
        private const val MaximumPending = 16
        private const val RememberedStreams = 32
    }
}
