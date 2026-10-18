package com.visualworkbench.android.precision

import android.view.MotionEvent
import com.visualworkbench.android.input.PenSample
import com.visualworkbench.android.input.PointerPacket
import com.visualworkbench.android.input.PointerTool
import com.visualworkbench.shared.Point
import com.visualworkbench.shared.PrecisionController

/** Only coordinates change. Accepted times, pressure, tilt, orientation and IDs
 * survive intact. Predictions use this same mapping but never move the aim. */
internal fun PrecisionController.routePen(samples: List<PenSample>): List<PenSample> {
    val points = route(samples.map { Point(it.x, it.y) })
    return samples.zip(points) { sample, point -> sample.copy(x = point.x, y = point.y) }
}

internal class PrecisionButtons(private val precision: PrecisionController, private val allowed: () -> Boolean) {
    // The current controller supplies its full existing canvas admission guard,
    // including modal/AI/result readiness; this helper owns no editor authority.
    private var pressedSince: Long? = null
    fun reset() { pressedSince = null }
    fun accept(packet: PointerPacket) {
        if (packet.canceled) { reset(); return }
        if (!allowed()) { reset(); return }
        val sample = packet.frames.lastOrNull()?.pointers?.firstOrNull {
            it.tool == PointerTool.Pen || it.tool == PointerTool.Eraser
        } ?: return
        if (precision.baseCamera() == null) return
        if (sample.buttons and MotionEvent.BUTTON_STYLUS_PRIMARY != 0) {
            if (pressedSince == null) pressedSince = sample.timeMs
            // A contact transition cancels the preceding canvas gesture. Keep
            // the actual physical press clock across hover -> contact, so a
            // long hold cannot become a short click merely by touching down.
            precision.barrel(true, checkNotNull(pressedSince))
        } else {
            precision.barrel(false, sample.timeMs)
            pressedSince = null
        }
    }
}
