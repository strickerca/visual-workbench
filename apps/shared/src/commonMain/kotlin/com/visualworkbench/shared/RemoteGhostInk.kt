package com.visualworkbench.shared

import kotlinx.coroutines.flow.MutableStateFlow

public data class GhostPoint(
    public val x: Float, public val y: Float, public val pressure: Float,
    public val localNanos: Long, public val predicted: Boolean,
    public val anchor: Boolean = false,
)
public data class GhostSegment(
    public val binding: RemoteTargetBinding,
    public val firstInputSequence: ULong, public val lastInputSequence: ULong,
    public val createdLocalNanos: Long, public val points: List<GhostPoint>,
)
public data class VisibleGhost(public val segment: GhostSegment, public val alpha: Float,
    public val acknowledgedLocalNanos: Long? = null)

/** All time arguments use the same phone-local monotonic clock (System.nanoTime
 * on Android). Input sequence is assigned by actual transport admission, never
 * by prediction. acknowledge is called ONLY after native retained-ticket
 * validation; this class cannot establish that authority or an editor effect. */
public class RemoteGhostInk {
    private data class Entry(val segment: GhostSegment, val acknowledgedAt: Long? = null)
    private data class Store(
        val binding: RemoteTargetBinding? = null, val entries: List<Entry> = emptyList(),
        val clock: Long = 0, val ackSequence: ULong = 0uL,
        val frameId: ULong = 0uL, val ticket: ULong = 0uL,
    )
    private val store = MutableStateFlow(Store())

    public fun activate(binding: RemoteTargetBinding, nowLocalNanos: Long): Boolean {
        if (!validBinding(binding) || nowLocalNanos < 0) return false
        while (true) {
            val old = store.value
            if (nowLocalNanos < old.clock) return false
            // Repeated activation never resets an existing segment's expiry.
            val next = if (old.binding == binding) old.copy(clock = nowLocalNanos)
                else Store(binding = binding, clock = nowLocalNanos)
            if (store.compareAndSet(old, next)) return true
        }
    }

    public fun add(segment: GhostSegment): Boolean = change(segment, update = false)
    public fun update(segment: GhostSegment): Boolean = change(segment, update = true)

    private fun change(segment: GhostSegment, update: Boolean): Boolean {
        if (!validSegment(segment)) return false
        val copied = segment.copy(points = segment.points.toList())
        while (true) {
            val old = store.value
            if (old.binding != copied.binding) return false
            // Predictions' future sample times never move the expiry clock.
            val now = maxOf(old.clock, copied.createdLocalNanos,
                copied.points.last { !it.predicted && !it.anchor }.localNanos)
            if (expired(copied.createdLocalNanos, now, HARD_EXPIRY_NANOS)) return false
            val live = old.entries.filterNot { expired(it.segment.createdLocalNanos, now, HARD_EXPIRY_NANOS) }
            val index = live.indexOfFirst { it.segment.firstInputSequence == copied.firstInputSequence }
            val nextEntries: List<Entry>
            if (update) {
                if (index < 0) return false
                val previous = live[index]
                // New predictions may replace old predictions. Actual admitted
                // samples form an immutable prefix; only new actual samples can
                // advance the last accepted sequence. Creation time stays fixed.
                val actual = copied.points.filterNot { it.predicted || it.anchor }
                val priorActual = previous.segment.points.filterNot { it.predicted || it.anchor }
                if (copied.createdLocalNanos != previous.segment.createdLocalNanos ||
                    copied.points.filter { it.anchor } != previous.segment.points.filter { it.anchor } ||
                    copied.lastInputSequence < previous.segment.lastInputSequence ||
                    actual.size < priorActual.size || actual.take(priorActual.size) != priorActual ||
                    (actual.size == priorActual.size && copied.lastInputSequence != previous.segment.lastInputSequence) ||
                    (actual.size > priorActual.size && copied.lastInputSequence <= previous.segment.lastInputSequence)) return false
                // Extending a previously covered segment needs a newer host ack.
                val covered = copied.lastInputSequence <= old.ackSequence
                nextEntries = live.mapIndexed { at, entry ->
                    if (at == index) Entry(copied, if (covered) previous.acknowledgedAt else null) else entry
                }
            } else {
                if (index >= 0 || live.size >= MAX_SEGMENTS || copied.firstInputSequence <= old.ackSequence ||
                    live.any { copied.firstInputSequence <= it.segment.lastInputSequence && copied.lastInputSequence >= it.segment.firstInputSequence }) return false
                nextEntries = live + Entry(copied)
            }
            if (nextEntries.sumOf { it.segment.points.size } > MAX_TOTAL_POINTS) return false
            if (store.compareAndSet(old, old.copy(clock = now, entries = nextEntries))) return true
        }
    }

    /** frameId/ticket are identities supplied by the validated retained native
     * frame receipt. A duplicate/older frame or receipt cannot restart a fade. */
    public fun acknowledge(
        binding: RemoteTargetBinding, lastInputSequenceApplied: ULong,
        frameId: ULong, ticket: ULong, callbackLocalNanos: Long,
    ): Boolean {
        if (frameId == 0uL || ticket == 0uL || callbackLocalNanos < 0) return false
        while (true) {
            val old = store.value
            if (old.binding != binding || frameId <= old.frameId ||
                ticket == old.ticket || lastInputSequenceApplied < old.ackSequence) return false
            // A valid callback can reach the controller after an overlay tick.
            // Its original arrival time still starts the fade, without moving
            // the local expiry clock backwards.
            val now = maxOf(old.clock, callbackLocalNanos)
            if (old.entries.any { it.segment.lastInputSequence <= lastInputSequenceApplied &&
                    it.segment.createdLocalNanos > callbackLocalNanos }) return false
            val next = old.copy(clock = now, ackSequence = lastInputSequenceApplied,
                frameId = frameId, ticket = ticket, entries = old.entries.mapNotNull { entry ->
                    if (expired(entry.segment.createdLocalNanos, now, HARD_EXPIRY_NANOS)) null
                    else if (entry.acknowledgedAt == null && entry.segment.lastInputSequence <= lastInputSequenceApplied)
                        entry.copy(acknowledgedAt = callbackLocalNanos) else entry
                })
            if (store.compareAndSet(old, next)) return true
        }
    }

    /** Immediate clear on revoke/background/scope retirement. An old owner
     * retiring its binding cannot erase a subsequently activated new binding. */
    public fun retire(binding: RemoteTargetBinding? = null): Unit {
        while (true) {
            val old = store.value
            if (binding != null && binding != old.binding) return
            if (store.compareAndSet(old, Store(clock = old.clock))) return
        }
    }

    public fun snapshot(nowLocalNanos: Long): List<VisibleGhost> {
        while (true) {
            val old = store.value
            if (nowLocalNanos < old.clock || nowLocalNanos < 0) return emptyList()
            val live = old.entries.filterNot { entry ->
                expired(entry.segment.createdLocalNanos, nowLocalNanos, HARD_EXPIRY_NANOS) ||
                    (entry.acknowledgedAt?.let { expired(it, nowLocalNanos, ACK_FADE_NANOS) } == true)
            }
            if (!store.compareAndSet(old, old.copy(clock = nowLocalNanos, entries = live))) continue
            return live.map { entry -> VisibleGhost(entry.segment.copy(points = entry.segment.points.toList()),
                entry.acknowledgedAt?.let { (1.0 - (nowLocalNanos - it).toDouble() / ACK_FADE_NANOS).toFloat().coerceIn(0f, 1f) } ?: 1f, entry.acknowledgedAt) }
        }
    }

    private fun validSegment(segment: GhostSegment): Boolean {
        val points = segment.points
        if (!validBinding(segment.binding) || segment.firstInputSequence == 0uL ||
            segment.lastInputSequence < segment.firstInputSequence || segment.createdLocalNanos < 0 ||
            points.isEmpty() || points.size > MAX_POINTS_PER_SEGMENT || points.none { !it.predicted && !it.anchor } ||
            points.count { !it.predicted && !it.anchor }.toULong() > segment.lastInputSequence - segment.firstInputSequence + 1uL) return false
        var predicted = false
        var lastTime = 0L
        for ((index, point) in points.withIndex()) {
            if (!point.x.isFinite() || !point.y.isFinite() || !point.pressure.isFinite() ||
                point.pressure !in 0f..1f || point.localNanos < lastTime ||
                (point.localNanos >= segment.createdLocalNanos && point.localNanos - segment.createdLocalNanos >= HARD_EXPIRY_NANOS) ||
                (point.anchor && (index != 0 || point.predicted || point.localNanos > segment.createdLocalNanos)) ||
                (predicted && !point.predicted)) return false
            predicted = predicted || point.predicted
            lastTime = point.localNanos
        }
        return true
    }
    private fun validBinding(binding: RemoteTargetBinding): Boolean =
        binding.connectionEpoch != 0uL && binding.sourceGeneration != 0uL &&
            binding.captureSessionId.isNotBlank() && binding.captureSessionId.length <= 128 &&
            binding.targetToken.isNotBlank() && binding.targetToken.length <= 256 &&
            binding.geometryRevision != 0u && binding.inputSessionId.isNotBlank() && binding.inputSessionId.length <= 128

    private fun expired(start: Long, now: Long, lifetime: Long): Boolean = now < start || now - start >= lifetime

    public companion object {
        public const val ACK_FADE_NANOS: Long = 150_000_000
        public const val HARD_EXPIRY_NANOS: Long = 500_000_000
        public const val MAX_SEGMENTS: Int = 64
        public const val MAX_POINTS_PER_SEGMENT: Int = 256
        public const val MAX_TOTAL_POINTS: Int = 4096
    }
}
