package com.visualworkbench.shared

/** UI-thread-owned tapped polygon. A drag adjusts only its current vertex;
 * real contact UP freezes that vertex. It never samples a lasso or writes data.
 * The owning SelectionInteraction retains the immutable binding/DOWN camera. */
internal class SelectionPolygon(private val limit: Int = 16_384) {
    private val vertices: MutableList<Point> = mutableListOf()
    private var contact: Point? = null
    val contacting: Boolean get() = contact != null
    val count: Int get() = vertices.size

    init { require(limit in 3..16_384) }

    fun begin(point: Point): Boolean {
        if (contact != null || vertices.size >= limit || !valid(point)) return false
        contact = point
        return true
    }
    fun move(point: Point): Boolean {
        if (contact == null || !valid(point)) return false
        contact = point
        return true
    }
    fun end(): Boolean {
        val point = contact ?: return false
        contact = null
        if (vertices.lastOrNull() != point) vertices += point
        return true
    }
    fun undo(): Boolean {
        if (contact != null || vertices.isEmpty()) return false
        vertices.removeAt(vertices.lastIndex)
        return true
    }
    fun preview(): List<Point> = vertices.toList() + listOfNotNull(contact)
    fun completed(): List<Point>? = if (contact == null && vertices.size >= 3) vertices.toList() else null
    private fun valid(point: Point): Boolean = point.x.isFinite() && point.y.isFinite() &&
        kotlin.math.abs(point.x) <= 8_388_608.0 && kotlin.math.abs(point.y) <= 8_388_608.0
}
