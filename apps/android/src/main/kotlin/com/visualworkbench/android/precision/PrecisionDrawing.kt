package com.visualworkbench.android.precision

import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import com.visualworkbench.android.editor.CanvasScene
import com.visualworkbench.android.editor.EditorDrawing
import com.visualworkbench.shared.*

/** Reuses the same immutable saved/wet paths and source bitmap. No screenshot,
 * extra full-size bitmap, provider read or alternate ink geometry is created. */
internal object PrecisionDrawing {
    fun draw(canvas:Canvas,scene:CanvasScene) {
        EditorDrawing.draw(canvas,scene)
        val frame=scene.precision
        for(pane in listOfNotNull(frame.lens,frame.loupe)) {
            val r=pane.rectangle
            canvas.save()
            try {
                canvas.clipRect(r.x.toFloat(),r.y.toFloat(),(r.x+r.width).toFloat(),(r.y+r.height).toFloat())
                EditorDrawing.draw(canvas,scene.copy(matrix=pane.matrix,camera=scene.camera.copy(scale=pane.scale),
                    selected=null,hover=null,peerViewport=emptyList(),precision=PrecisionFrame(),smoothBackground=pane.smooth))
            } finally {canvas.restore()}
            val paint=Paint(Paint.ANTI_ALIAS_FLAG).apply {color=if(pane.editable)0xff74e5c2.toInt() else 0xffecc769.toInt();style=Paint.Style.STROKE;strokeWidth=2*scene.density}
            canvas.drawRect(RectF(r.x.toFloat(),r.y.toFloat(),(r.x+r.width).toFloat(),(r.y+r.height).toFloat()),paint)
            if(!pane.editable){val x=(r.x+r.width/2).toFloat();val y=(r.y+r.height/2).toFloat();canvas.drawLine(x-9,y,x+9,y,paint);canvas.drawLine(x,y-9,x,y+9,paint)}
            if(!pane.smooth)grid(canvas,pane)
        }
        // Mini-map is the base viewport outline plus the lens footprint, never a
        // second input surface. It cannot alter the camera or send a viewport.
        if(frame.overview.size==4){
            val scale=minOf(140.0/scene.camera.viewportWidth,100.0/scene.camera.viewportHeight)
            val left=scene.camera.viewportWidth-140-8;val top=scene.camera.viewportHeight-100-8
            val p=Paint(Paint.ANTI_ALIAS_FLAG).apply{color=0xe0101822.toInt()}
            canvas.drawRect(left.toFloat(),top.toFloat(),(left+140).toFloat(),(top+100).toFloat(),p)
            p.color=0xff74e5c2.toInt();p.style=Paint.Style.STROKE;p.strokeWidth=scene.density
            canvas.drawRect(left.toFloat(),top.toFloat(),(left+scene.camera.viewportWidth*scale).toFloat(),(top+scene.camera.viewportHeight*scale).toFloat(),p)
            val path=Path();frame.overview.forEachIndexed{i,v->val x=(left+v.x*scale).toFloat();val y=(top+v.y*scale).toFloat();if(i==0)path.moveTo(x,y)else path.lineTo(x,y)};path.close()
            canvas.save();try{canvas.clipRect(left.toFloat(),top.toFloat(),(left+140).toFloat(),(top+100).toFloat());canvas.drawPath(path,p)}finally{canvas.restore()}
        }
    }
    private fun grid(canvas:Canvas,pane:MagnifierPane){
        val lines=precisionPixelGrid(pane)
        val paint=Paint().apply{color=0x506e8294;strokeWidth=1f}
        canvas.save();try{val r=pane.rectangle;canvas.clipRect(r.x.toFloat(),r.y.toFloat(),(r.x+r.width).toFloat(),(r.y+r.height).toFloat())
            lines.forEach{(a,b)->canvas.drawLine(a.x.toFloat(),a.y.toFloat(),b.x.toFloat(),b.y.toFloat(),paint)}
        }finally{canvas.restore()}
    }
}
