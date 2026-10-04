package com.visualworkbench.android.editor

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject

internal enum class EditorTool(val label: String, val shortLabel: String) {
    Pen("Pen", "Pen"), Highlighter("Highlighter", "High"), Marker("Marker brush", "Mark"), Callout("Numbered marker", "1 2"),
    Line("Line", "Line"), Arrow("Arrow", "Arrow"), Rectangle("Rectangle", "Rect"),
    Ellipse("Ellipse", "Oval"), Text("Text", "Text"), Eraser("Eraser modes", "Erase"), Select("Select and transform", "Select"),
}

internal data class PressureCurve(val x1: Float = 0.25f, val y1: Float = 0.25f, val x2: Float = 0.75f, val y2: Float = 0.75f) {
    init {
        require(listOf(x1, y1, x2, y2).all { it.isFinite() && it in 0f..1f } && x1 <= x2 && y1 <= y2)
    }
    fun withControl(index: Int, value: Float): PressureCurve = when (index) {
        0 -> copy(x1 = value.coerceIn(0f, x2))
        1 -> copy(y1 = value.coerceIn(0f, y2))
        2 -> copy(x2 = value.coerceIn(x1, 1f))
        3 -> copy(y2 = value.coerceIn(y1, 1f))
        else -> this
    }
}

internal data class BrushPreference(
    val width: Double = 6.0,
    val colorArgb: Long = 0xff51c7c2,
    val stabilization: Float = 0.15f,
    val pressure: PressureCurve = PressureCurve(),
) {
    init { require(width.isFinite() && width in 0.25..256.0 && stabilization.isFinite() && stabilization in 0f..1f && colorArgb in 0..0xffffffff) }
}

internal data class EditorPreferences(
    val drawWithFinger: Boolean = false,
    val leftHanded: Boolean = false,
    val darkTheme: Boolean = true,
    val reducedMotion: Boolean = false,
    val railOffset: Float = 0.25f,
    val brushes: Map<EditorTool, BrushPreference> = mapOf(
        EditorTool.Pen to BrushPreference(),
        EditorTool.Highlighter to BrushPreference(24.0, 0x66ffc857, 0.08f),
        EditorTool.Marker to BrushPreference(12.0, 0xffec8792, 0.10f),
    ),
) {
    init { require(railOffset.isFinite() && railOffset in 0f..1f) }
    fun brush(tool: EditorTool): BrushPreference = brushes[tool] ?: brushes.getValue(EditorTool.Pen)
    fun withBrush(tool: EditorTool, brush: BrushPreference): EditorPreferences = copy(brushes = brushes + ((if (tool in brushes) tool else EditorTool.Pen) to brush))
}

/** Preference disk reads and synchronous durable writes stay off the input thread. */
internal class EditorPreferencesStore(context: Context) {
    private val context = context.applicationContext
    suspend fun read(): EditorPreferences = withContext(Dispatchers.IO) {
        val defaults = EditorPreferences()
        val raw = context.getSharedPreferences("editor-preferences", Context.MODE_PRIVATE).getString("settings", null)
            ?: return@withContext defaults
        if (raw.length > 16_384) return@withContext defaults
        try {
            val value = JSONObject(raw)
            val brushes = defaults.brushes.mapValues { (tool, fallback) ->
                val item = value.optJSONObject(tool.name) ?: return@mapValues fallback
                val curve = item.optJSONObject("pressure")
                BrushPreference(item.optDouble("width", fallback.width), item.optLong("argb", fallback.colorArgb),
                    item.optDouble("stabilization", fallback.stabilization.toDouble()).toFloat(),
                    if (curve == null) fallback.pressure else PressureCurve(curve.getDouble("x1").toFloat(), curve.getDouble("y1").toFloat(),
                        curve.getDouble("x2").toFloat(), curve.getDouble("y2").toFloat()))
            }
            EditorPreferences(value.optBoolean("drawWithFinger"), value.optBoolean("leftHanded"),
                value.optBoolean("darkTheme", true), value.optBoolean("reducedMotion"), value.optDouble("railOffset", 0.25).toFloat(), brushes)
        } catch (_: IllegalArgumentException) { defaults }
        catch (_: org.json.JSONException) { defaults }
    }
    suspend fun write(settings: EditorPreferences): Unit = withContext(Dispatchers.IO) {
        val json = JSONObject().put("schema", 1).put("drawWithFinger", settings.drawWithFinger)
            .put("leftHanded", settings.leftHanded).put("darkTheme", settings.darkTheme)
            .put("reducedMotion", settings.reducedMotion).put("railOffset", settings.railOffset.toDouble())
        settings.brushes.forEach { (tool, brush) ->
            json.put(tool.name, JSONObject().put("width", brush.width).put("argb", brush.colorArgb)
                .put("stabilization", brush.stabilization.toDouble()).put("pressure", JSONObject()
                    .put("x1", brush.pressure.x1.toDouble()).put("y1", brush.pressure.y1.toDouble())
                    .put("x2", brush.pressure.x2.toDouble()).put("y2", brush.pressure.y2.toDouble())))
        }
        check(context.getSharedPreferences("editor-preferences", Context.MODE_PRIVATE).edit().putString("settings", json.toString()).commit()) {
            "Settings could not be saved"
        }
    }
}
