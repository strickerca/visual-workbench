package com.visualworkbench.desktop.precision

import com.visualworkbench.shared.Point
import com.visualworkbench.shared.Rect
import com.visualworkbench.shared.Transform
import kotlin.math.abs

/** Resize handles have a physical screen-pixel target regardless of lens
 * magnification. Edits still use the separately mapped document-space contact. */
internal fun precisionResizeHit(bounds: Rect, physical: Transform, raw: Point, radius: Double): Boolean {
    if (!radius.isFinite() || radius < 0.0 ||
        !listOf(bounds.x, bounds.y, bounds.width, bounds.height, raw.x, raw.y,
            physical.a, physical.b, physical.c, physical.d, physical.e, physical.f).all(Double::isFinite)) return false
    val x = bounds.x + bounds.width
    val y = bounds.y + bounds.height
    val screenX = physical.a * x + physical.c * y + physical.e
    val screenY = physical.b * x + physical.d * y + physical.f
    return screenX.isFinite() && screenY.isFinite() && abs(raw.x - screenX) <= radius && abs(raw.y - screenY) <= radius
}
