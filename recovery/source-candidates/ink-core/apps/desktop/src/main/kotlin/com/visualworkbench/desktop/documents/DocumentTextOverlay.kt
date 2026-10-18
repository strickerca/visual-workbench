package com.visualworkbench.desktop.documents

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import com.visualworkbench.shared.*

@Composable internal fun DocumentTextOverlay(controller:DocumentController,core:WorkbenchCore,camera:Camera) {
    val state by controller.state.collectAsState();if(state.descriptor==null)return
    var drag by remember(state.page,state.descriptor?.revision?.stateHash){mutableStateOf<Rect?>(null)}
    val enabled=(state.textTool||state.regionTool)&&!state.busy
    Canvas(Modifier.fillMaxSize().then(if(enabled)Modifier.pointerInput(camera,state.page,state.regionTool){awaitEachGesture{
        val down=awaitFirstDown();down.consume();val first=core.mapPoints(camera,true,listOf(Point(down.position.x.toDouble(),down.position.y.toDouble()))).single()
        var finished=false
        try{while(true){val event=awaitPointerEvent();val point=event.changes.firstOrNull{it.id==down.id}?:break;val last=core.mapPoints(camera,true,listOf(Point(point.position.x.toDouble(),point.position.y.toDouble()))).single();drag=Rect(minOf(first.x,last.x),minOf(first.y,last.y),kotlin.math.abs(last.x-first.x),kotlin.math.abs(last.y-first.y));point.consume();if(!point.pressed){finished=true;break}}}
        finally{val region=drag;drag=null;if(finished&&region!=null&&region.width>0&&region.height>0){if(state.regionTool)controller.region(region)else controller.select(region)}}
    }}else Modifier)) {
        fun box(rect:Rect,color:Color){val points=core.mapPoints(camera,false,listOf(Point(rect.x,rect.y),Point(rect.x+rect.width,rect.y),Point(rect.x+rect.width,rect.y+rect.height),Point(rect.x,rect.y+rect.height)));val path=Path().apply{points.forEachIndexed{i,p->if(i==0)moveTo(p.x.toFloat(),p.y.toFloat())else lineTo(p.x.toFloat(),p.y.toFloat())};close()};drawPath(path,color.copy(alpha=.18f));drawPath(path,color,style=Stroke(1.5f))}
        state.glyphs.mapNotNull{it.bounds}.forEach{box(it,Color(0xff82b9ff))};state.regions.filter{it.page==state.page}.forEach{box(it.rectangle,Color(0xffe68c87))};drag?.let{box(it,Color(0xff81e0c8))}
    }
}
