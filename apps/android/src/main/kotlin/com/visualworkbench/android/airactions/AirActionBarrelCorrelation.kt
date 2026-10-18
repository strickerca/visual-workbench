package com.visualworkbench.android.airactions

import com.visualworkbench.shared.AirActionRouter

/** An ordinary canvas cancellation retires Air presses without losing an
 * observed physical barrel hold/release. Full focus/input ownership loss clears
 * this observation. PrecisionButtons still owns the actual barrel command. */
internal class AirActionBarrelCorrelation {
    private var active = false
    private var observedAt = -1L
    private var releasedAt = -1L

    fun observe(down: Boolean, timeMs: Long) {
        if (timeMs < 0) { clear(); return }
        if (down) { active = true; observedAt = timeMs }
        else if (active) { active = false; releasedAt = timeMs }
    }
    fun restore(router: AirActionRouter, nowMs: Long) {
        if (active && observedAt >= 0 && nowMs - observedAt in 0L..AirActionRouter.MaximumPressMs) {
            router.barrel(true, observedAt)
        } else if (!active && releasedAt >= 0 && nowMs - releasedAt in 0L..AirActionRouter.ResolutionMs) {
            router.barrel(true, releasedAt)
            router.barrel(false, releasedAt)
        }
    }
    fun clear() { active = false; observedAt = -1L; releasedAt = -1L }
}
