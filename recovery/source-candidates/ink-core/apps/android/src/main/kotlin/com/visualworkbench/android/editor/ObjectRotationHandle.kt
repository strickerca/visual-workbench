package com.visualworkbench.android.editor

import android.graphics.Canvas
import android.graphics.Paint
import com.visualworkbench.shared.ObjectRotationHandle
import com.visualworkbench.shared.RenderItem
import com.visualworkbench.shared.objectRotationAllowed
import com.visualworkbench.shared.objectRotationHandle

/** The painter and input handler use this same physical-pixel knob. */
internal fun androidRotationHandle(scene: CanvasScene, item: RenderItem): ObjectRotationHandle? =
    if(objectRotationAllowed(listOf(item)))objectRotationHandle(EditorDrawing.screenBounds(scene,item),32.0*scene.density)else null

internal fun drawAndroidRotationHandle(canvas: Canvas, scene: CanvasScene, item: RenderItem) {
    val handle=androidRotationHandle(scene,item)?:return
    val paint=Paint(Paint.ANTI_ALIAS_FLAG).apply{color=0xff087dba.toInt();style=Paint.Style.STROKE;strokeWidth=2*scene.density}
    canvas.drawLine(handle.anchor.x.toFloat(),handle.anchor.y.toFloat(),handle.knob.x.toFloat(),handle.knob.y.toFloat(),paint)
    paint.style=Paint.Style.FILL
    canvas.drawCircle(handle.knob.x.toFloat(),handle.knob.y.toFloat(),7*scene.density,paint)
    paint.color=android.graphics.Color.WHITE;paint.style=Paint.Style.STROKE;paint.strokeWidth=scene.density
    canvas.drawCircle(handle.knob.x.toFloat(),handle.knob.y.toFloat(),4*scene.density,paint)
}
