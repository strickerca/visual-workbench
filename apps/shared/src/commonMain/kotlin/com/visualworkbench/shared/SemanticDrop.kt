package com.visualworkbench.shared

/** Classify a tap vs an explicit numbered-box drag only. This does not implement
 * semantic distance/snapping; snap() remains the canonical native authority.
 * Inputs are D coordinates and the actual D-to-physical-viewport matrix. */
public fun semanticDropBox(first: Point, last: Point, documentToPhysical: Transform): Rect? {
    val m = documentToPhysical
    if (listOf(first.x, first.y, last.x, last.y, m.a, m.b, m.c, m.d, m.e, m.f).any { !it.isFinite() }) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    val dx = last.x - first.x; val dy = last.y - first.y
    val sx = m.a * dx + m.c * dy; val sy = m.b * dx + m.d * dy
    if (!dx.isFinite() || !dy.isFinite() || !sx.isFinite() || !sy.isFinite()) throw WorkflowFailure(WorkflowFailureKind.Invalid)
    if (kotlin.math.hypot(sx, sy) < 4.0 || dx == 0.0 || dy == 0.0) return null
    return Rect(minOf(first.x, last.x), minOf(first.y, last.y), kotlin.math.abs(dx), kotlin.math.abs(dy))
}
