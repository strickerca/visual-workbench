package com.visualworkbench.android.precision

import com.visualworkbench.android.editor.CanvasScene
import com.visualworkbench.android.editor.EditorDrawing
import com.visualworkbench.shared.Point
import com.visualworkbench.shared.PrecisionFrame
import com.visualworkbench.shared.RenderItem
import com.visualworkbench.shared.Transform
import kotlin.math.hypot

/** Reuse the actual painted/hit geometry in physical coordinates. The lens is
 * a uniform magnification of the canonical camera; camera.scale is consumed by
 * screen-constant strokes, while matrix remains the full D-to-physical affine.
 * Keep the real viewport: clipping a remapped base point to that viewport would
 * incorrectly reject content visible only through a lens near the page edge. */
internal fun precisionHitScene(scene: CanvasScene, physical: Transform): CanvasScene {
    require(listOf(physical.a, physical.b, physical.c, physical.d, physical.e, physical.f).all(Double::isFinite))
    val scale = hypot(physical.a, physical.b)
    require(scale.isFinite() && scale > 0.0)
    return scene.copy(matrix = physical, camera = scene.camera.copy(scale = scale),
        precision = PrecisionFrame(), hover = null)
}

/** The caller passes the raw accepted contact, never the base-remapped sample.
 * Return the same corner index used by the editor's opposite-corner anchor. */
internal fun precisionResizeHandle(scene: CanvasScene, item: RenderItem, raw: Point, radius: Float): Int? {
    require(raw.x.isFinite() && raw.y.isFinite() && radius.isFinite() && radius >= 0f)
    val handles = EditorDrawing.screenBounds(scene, item)
    val near = handles.indices.minByOrNull { hypot(handles[it].x - raw.x, handles[it].y - raw.y) } ?: return null
    return near.takeIf { hypot(handles[it].x - raw.x, handles[it].y - raw.y) <= radius }
}
