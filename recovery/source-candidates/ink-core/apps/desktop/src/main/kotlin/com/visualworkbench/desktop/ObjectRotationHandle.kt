package com.visualworkbench.desktop

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import com.visualworkbench.shared.*

/** Canvas offsets are physical pixels, matching the core camera mapping. */
internal fun desktopRotationHandle(core: WorkbenchCore, camera: Camera, selected: List<RenderItem>,
    previews: Map<String,Transform> = emptyMap()): ObjectRotationHandle? {
    if(!objectRotationAllowed(selected))return null
    val bounds=union(selected.map{previewBounds(it,previews[it.objectId])})?:return null
    val corners=listOf(Point(bounds.x,bounds.y),Point(bounds.x+bounds.width,bounds.y),
        Point(bounds.x+bounds.width,bounds.y+bounds.height),Point(bounds.x,bounds.y+bounds.height))
    return objectRotationHandle(core.mapPoints(camera,false,corners),32.0)
}
internal fun DrawScope.drawDesktopRotationHandle(handle: ObjectRotationHandle) {
    val anchor=Offset(handle.anchor.x.toFloat(),handle.anchor.y.toFloat())
    val knob=Offset(handle.knob.x.toFloat(),handle.knob.y.toFloat())
    drawLine(Color(0xff81e0c8),anchor,knob,2f)
    drawCircle(Color(0xff81e0c8),7f,knob)
    drawCircle(Color(0xff0c1118),4f,knob,style=Stroke(1f))
}
