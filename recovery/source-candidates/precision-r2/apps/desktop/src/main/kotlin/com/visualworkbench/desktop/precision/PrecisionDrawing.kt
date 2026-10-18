package com.visualworkbench.desktop.precision

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipRect
import com.visualworkbench.shared.*

internal fun DrawScope.drawPrecision(frame:PrecisionFrame,camera:Camera,document:(Transform,Double,Boolean)->Unit) {
    for(pane in listOfNotNull(frame.lens,frame.loupe)) {
        val r=pane.rectangle
        clipRect(r.x.toFloat(),r.y.toFloat(),(r.x+r.width).toFloat(),(r.y+r.height).toFloat()) {
            document(pane.matrix,pane.scale,pane.smooth)
            if(!pane.smooth)precisionPixelGrid(pane).forEach{(a,b)->drawLine(Color(0x506e8294),Offset(a.x.toFloat(),a.y.toFloat()),Offset(b.x.toFloat(),b.y.toFloat()),1f)}
        }
        val color=if(pane.editable)Color(0xff74e5c2) else Color(0xffecc769)
        drawRect(color,Offset(r.x.toFloat(),r.y.toFloat()),Size(r.width.toFloat(),r.height.toFloat()),style=Stroke(2f))
        if(!pane.editable){val x=(r.x+r.width/2).toFloat();val y=(r.y+r.height/2).toFloat();drawLine(color,Offset(x-9,y),Offset(x+9,y));drawLine(color,Offset(x,y-9),Offset(x,y+9))}
    }
    if(frame.overview.size==4){
        val scale=minOf(140.0/camera.viewportWidth,100.0/camera.viewportHeight)
        val left=camera.viewportWidth-148;val top=camera.viewportHeight-108
        drawRect(Color(0xe0101822),Offset(left.toFloat(),top.toFloat()),Size(140f,100f))
        drawRect(Color(0xff74e5c2),Offset(left.toFloat(),top.toFloat()),Size((camera.viewportWidth*scale).toFloat(),(camera.viewportHeight*scale).toFloat()),style=Stroke(1f))
        val path=Path();frame.overview.forEachIndexed{i,p->val x=(left+p.x*scale).toFloat();val y=(top+p.y*scale).toFloat();if(i==0)path.moveTo(x,y)else path.lineTo(x,y)};path.close()
        clipRect(left.toFloat(),top.toFloat(),(left+140).toFloat(),(top+100).toFloat()){drawPath(path,Color(0xff74e5c2),style=Stroke(1f))}
    }
}
