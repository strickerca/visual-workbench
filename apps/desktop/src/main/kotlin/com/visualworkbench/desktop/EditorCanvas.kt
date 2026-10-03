@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.*
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.*
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.layout.onSizeChanged
import com.visualworkbench.shared.*
import com.visualworkbench.shared.Outline
import com.visualworkbench.shared.Shape
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

internal enum class PaintKind { ObjectFill, StrokeFill, Outline }
internal data class PaintPass(val path:Path,val kind:PaintKind,val documentPath:Path?=null)
internal data class CanvasGeometry(val passes:List<PaintPass>)
internal data class GeometryFrame<T>(val binding:RenderBinding,val document:DocumentSnapshot,val entries:Map<String,T>) {
    fun current(state:EditorState):GeometryFrame<T>? = takeIf{geometryMatches(binding,state)}
}
internal data class OverlayFrame(val binding:RenderBinding,val source:List<RenderItem>,val entries:Map<String,CanvasGeometry>)
internal fun overlayItems(state:EditorState):List<RenderItem>{
    val document=state.document?:return emptyList();val binding=document.binding(state.projectEpoch)
    val items=linkedMapOf<String,RenderItem>()
    state.peerFrame?.takeIf{it.binding==binding}?.previews?.items?.forEach{items[it.objectId]=it}
    for((id,style)in state.previewStyles){val item=items[id]?:document.render.items.firstOrNull{it.objectId==id}?:continue;items[id]=item.copy(style=style)}
    state.wetStroke?.takeIf{wet->document.render.items.none{it.objectId==wet.objectId}}?.let{items[it.objectId]=it}
    return items.values.toList()
}
private fun color(rgba:UInt):Color = Color(((rgba and 255u) shl 24 or (rgba shr 8)).toInt())
internal fun Transform.matrix():Matrix = Matrix(floatArrayOf(a.toFloat(),b.toFloat(),0f,0f,c.toFloat(),d.toFloat(),0f,0f,0f,0f,1f,0f,e.toFloat(),f.toFloat(),0f,1f))
internal fun Path.transformed(value:Transform):Path = Path().also{it.addPath(this);it.transform(value.matrix())}
private fun Contours.path(checkCancelled:()->Unit):Path = Path().apply {
    fillType=PathFillType.NonZero
    var start=0
    for(end in ends){val stop=end.toInt();if(stop>start){moveTo(x[start]/256f,y[start]/256f);for(i in start+1 until stop){if(i%4096==0)checkCancelled();lineTo(x[i]/256f,y[i]/256f)};close()};start=stop}
}
private fun List<Outline>.path(checkCancelled:()->Unit):Path = Path().apply {
    fillType=PathFillType.NonZero
    for((index,command)in this@path.withIndex()){
        if(index%4096==0)checkCancelled()
        when(command){is Outline.Move->moveTo(command.x,command.y);is Outline.Line->lineTo(command.x,command.y);is Outline.Quad->quadraticTo(command.x1,command.y1,command.x,command.y);is Outline.Cubic->cubicTo(command.x1,command.y1,command.x2,command.y2,command.x,command.y);Outline.Close->close()}
    }
}
private fun polyline(points:List<Point>,closed:Boolean=false,checkCancelled:()->Unit={}):Path = Path().apply {
    points.firstOrNull()?.let{moveTo(it.x.toFloat(),it.y.toFloat())}
    for(index in 1 until points.size){if(index%4096==0)checkCancelled();val point=points[index];lineTo(point.x.toFloat(),point.y.toFloat())}
    if(closed)close()
}
private fun Rect.compose():androidx.compose.ui.geometry.Rect = androidx.compose.ui.geometry.Rect(x.toFloat(),y.toFloat(),(x+width).toFloat(),(y+height).toFloat())

/** Ordered passes mirror vw-raster: marker circle and box each composite their
 * own fill/outline, while an ink stroke is one compound NONZERO fill. */
internal fun prepareGeometry(item:RenderItem,markerText:TextLayout?=null,checkCancelled:()->Unit={}):CanvasGeometry {
    val passes=mutableListOf<PaintPass>()
    fun stroke(path:Path){if(item.style.width>0.0)passes+=PaintPass(path,PaintKind.Outline,if(item.style.screenConstantWidth)path.transformed(item.transform)else null)}
    fun outlined(path:Path){if(item.style.fill!=null)passes+=PaintPass(path,PaintKind.ObjectFill);stroke(path)}
    when(val shape=item.shape){
        is Shape.Stroke->passes+=PaintPass(item.contours.path(checkCancelled),PaintKind.StrokeFill)
        is Shape.Text->passes+=PaintPass(shape.outline.path(checkCancelled).apply{translate(Offset(shape.anchor.x.toFloat(),shape.anchor.y.toFloat()))},PaintKind.StrokeFill)
        is Shape.Line->stroke(polyline(shape.points,checkCancelled=checkCancelled))
        is Shape.Arrow->{
            stroke(polyline(shape.points,checkCancelled=checkCancelled))
            val end=shape.points.lastOrNull();val before=shape.points.dropLast(1).lastOrNull{it!=end}
            if(end!=null&&before!=null){val dx=end.x-before.x;val dy=end.y-before.y;val length=kotlin.math.hypot(dx,dy);val size=maxOf(8.0,item.style.width*4.0);val ux=dx/length;val uy=dy/length
                passes+=PaintPass(polyline(listOf(end,Point(end.x-ux*size-uy*size*.45,end.y-uy*size+ux*size*.45),Point(end.x-ux*size+uy*size*.45,end.y-uy*size-ux*size*.45)),true),PaintKind.StrokeFill)
            }
        }
        is Shape.Polygon->outlined(polyline(shape.points,true,checkCancelled))
        is Shape.Rectangle->outlined(Path().apply{addRect(shape.rectangle.compose())})
        is Shape.Ellipse->outlined(Path().apply{addOval(shape.rectangle.compose())})
        is Shape.Guide->stroke(Path().apply{addRect(shape.rectangle.compose())})
        is Shape.Marker->{
            val radius=maxOf(12.0,item.style.width*4.0)
            outlined(Path().apply{addOval(androidx.compose.ui.geometry.Rect((shape.point.x-radius).toFloat(),(shape.point.y-radius).toFloat(),(shape.point.x+radius).toFloat(),(shape.point.y+radius).toFloat()))})
            shape.rectangle?.takeIf{it.width>0&&it.height>0}?.let{outlined(Path().apply{addRect(it.compose())})}
            markerText?.let{layout->val glyphs=layout.glyphs.flatMap{it.outline}.path(checkCancelled).apply{translate(Offset((shape.point.x-layout.width/2).toFloat(),(shape.point.y+radius*1.15*.35).toFloat()))};passes+=PaintPass(glyphs,PaintKind.StrokeFill)}
        }
        is Shape.Result,Shape.Adjustment->error("Composite objects require the core composite display path")
    }
    return CanvasGeometry(passes)
}

/** Constant-width outlines are stroked in D after the object transform. Only
 * the camera acts on their pen width; even a shear/nonuniform scale cannot
 * multiply it. Fill geometry retains the object's complete transform. */
internal fun DrawScope.drawGeometry(item:RenderItem,geometry:CanvasGeometry,preview:Transform?,cameraScale:Double){
    val objectTransform=preview?:item.transform
    for(pass in geometry.passes){
        if(pass.kind==PaintKind.Outline&&item.style.screenConstantWidth){
            val documentPath=if(preview==null)checkNotNull(pass.documentPath)else pass.path.transformed(objectTransform)
            drawPath(documentPath,color(item.style.rgba),style=Stroke((item.style.width/cameraScale).toFloat(),cap=StrokeCap.Round,join=StrokeJoin.Round))
        }else withTransform({transform(objectTransform.matrix())}){
            when(pass.kind){
                PaintKind.ObjectFill->drawPath(pass.path,color(checkNotNull(item.style.fill)))
                PaintKind.StrokeFill->drawPath(pass.path,color(item.style.rgba))
                PaintKind.Outline->drawPath(pass.path,color(item.style.rgba),style=Stroke(item.style.width.toFloat(),cap=StrokeCap.Round,join=StrokeJoin.Round))
            }
        }
    }
}

@OptIn(ExperimentalComposeUiApi::class)
@Composable
fun EditorCanvas(controller:EditorController,state:EditorState,modifier:Modifier=Modifier){
    val latest by rememberUpdatedState(state)
    val focus=remember{FocusRequester()}
    var prepared by remember{mutableStateOf<GeometryFrame<CanvasGeometry>?>(null)}
    var overlay by remember{mutableStateOf<OverlayFrame?>(null)}
    val binding=state.document?.binding(state.projectEpoch)
    val requestedOverlay=remember(binding,state.peerFrame,state.previewStyles,state.wetStroke){overlayItems(state)}
    val latestOverlay by rememberUpdatedState(requestedOverlay)
    LaunchedEffect(binding,requestedOverlay){
        overlay=null
        val requested=binding?:return@LaunchedEffect
        try{
            val geometry=withContext(Dispatchers.Default){val context=currentCoroutineContext();requestedOverlay.associate{item->
                context.ensureActive();val marker=item.shape as? Shape.Marker
                val text=marker?.let{controller.core.layoutText(it.number.toString(),"Inter",(maxOf(12.0,item.style.width*4)*1.15).toFloat())}
                item.objectId to prepareGeometry(item,text){context.ensureActive()}
            }}
            if(geometryMatches(requested,latest)&&latestOverlay===requestedOverlay)overlay=OverlayFrame(requested,requestedOverlay,geometry)
        }catch(cancel:kotlinx.coroutines.CancellationException){throw cancel}
        catch(_:Exception){controller.report("An intermediate preview could not be drawn. Saved edits remain available.")}
    }
    LaunchedEffect(binding){
        prepared=null
        val document=state.document?:return@LaunchedEffect
        val requested=checkNotNull(binding)
        if(state.renderIssue!=null)return@LaunchedEffect
        try{
            val geometry=withContext(Dispatchers.Default){
                val context=currentCoroutineContext()
                document.render.items.associate{item->
                    context.ensureActive()
                    val marker=item.shape as? Shape.Marker
                    val text=marker?.let{controller.core.layoutText(it.number.toString(),"Inter",(maxOf(12.0,item.style.width*4)*1.15).toFloat())}
                    item.objectId to prepareGeometry(item,text){context.ensureActive()}
                }
            }
            val frame=GeometryFrame(requested,document,geometry)
            if(frame.current(latest)!=null){prepared=frame;controller.canvasPrepared(requested)}
        }catch(cancel:kotlinx.coroutines.CancellationException){throw cancel}
        catch(_:Exception){prepared=null;controller.canvasFailed(requested,"Canvas geometry could not be prepared. Reopen the project before editing; saved data is intact.")}
    }
    Canvas(modifier.fillMaxSize().clipToBounds().focusRequester(focus).onFocusChanged{controller.canvasFocus(it.isFocused)}.focusable()
        .onSizeChanged{controller.viewport(it.width.toDouble(),it.height.toDouble())}
        .onPointerEvent(PointerEventType.Scroll){event->val change=event.changes.firstOrNull()?:return@onPointerEvent;val delta=change.scrollDelta;if(event.keyboardModifiers.isCtrlPressed){controller.zoom(kotlin.math.exp(-delta.y.toDouble()*0.12),Point(change.position.x.toDouble(),change.position.y.toDouble()))}else{controller.pan(-delta.x*28.0,-delta.y*28.0)};change.consume()}
        .pointerInput(controller){awaitEachGesture{
            val down=awaitFirstDown(requireUnconsumed=false);focus.requestFocus()
            controller.pointerDown(Point(down.position.x.toDouble(),down.position.y.toDouble()),currentEvent.keyboardModifiers.isShiftPressed,down.type==PointerType.Touch||currentEvent.buttons.isTertiaryPressed||currentEvent.buttons.isSecondaryPressed,down.uptimeMillis);down.consume()
            var completed=false
            try{while(true){val event=awaitPointerEvent();val change=event.changes.firstOrNull{it.id==down.id}?:break;if(!change.pressed){controller.pointerUp(Point(change.position.x.toDouble(),change.position.y.toDouble()),change.uptimeMillis);completed=true;change.consume();break};controller.pointerMove(Point(change.position.x.toDouble(),change.position.y.toDouble()),change.uptimeMillis);change.consume()}}
            finally{if(completed)controller.cancelDrag()else controller.cancelInput()}
        }}
    ){
        drawRect(Color(0xff0c1118))
        val current=latest
        val frame=prepared?.current(current)?:return@Canvas
        val background=current.background?:return@Canvas
        val doc=frame.document
        val currentOverlay=overlay?.takeIf{it.binding==frame.binding&&it.source===latestOverlay}
        val replacements=currentOverlay?.source?.associateBy{it.objectId}.orEmpty()
        val originalIds=doc.render.items.mapTo(hashSetOf()){it.objectId}
        val displayed=doc.render.items.map{replacements[it.objectId]?:it}+replacements.values.filter{it.objectId !in originalIds}
        val camera=controller.core.cameraMatrix(current.view.camera)
        withTransform({transform(camera.matrix())}){
            drawRect(Color.White,size=Size(doc.width.toFloat(),doc.height.toFloat()))
            drawImage(background,filterQuality=FilterQuality.None)
            clipRect(0f,0f,doc.width.toFloat(),doc.height.toFloat()){
                for((_,items)in displayed.groupBy{it.layerId}){
                    val first=items.first();val paint=Paint().apply{alpha=first.layerOpacity.toFloat();blendMode=if(first.layerBlend=="multiply")BlendMode.Multiply else BlendMode.SrcOver}
                    drawContext.canvas.saveLayer(androidx.compose.ui.geometry.Rect(0f,0f,doc.width.toFloat(),doc.height.toFloat()),paint)
                    try{for(item in items)drawGeometry(item,checkNotNull(currentOverlay?.entries?.get(item.objectId)?:frame.entries[item.objectId]),current.preview[item.objectId],current.view.camera.scale)}finally{drawContext.canvas.restore()}
                }
            }
            val selected=doc.render.items.filter{it.objectId in current.selected}
            union(selected.map{previewBounds(it,current.preview[it.objectId])})?.let{bounds->val width=(1.0/current.view.camera.scale).toFloat();drawRect(Color(0xff81e0c8),Offset(bounds.x.toFloat(),bounds.y.toFloat()),Size(bounds.width.toFloat(),bounds.height.toFloat()),style=Stroke(width));val side=(8.0/current.view.camera.scale).toFloat();drawRect(Color(0xff81e0c8),Offset((bounds.x+bounds.width).toFloat()-side/2,(bounds.y+bounds.height).toFloat()-side/2),Size(side,side))}
            current.draft?.let{(a,b)->drawRect(color(current.color),Offset(minOf(a.x,b.x).toFloat(),minOf(a.y,b.y).toFloat()),Size(kotlin.math.abs(a.x-b.x).toFloat(),kotlin.math.abs(a.y-b.y).toFloat()),style=Stroke((1.0/current.view.camera.scale).toFloat()))}
            if(current.view.showPeerOutline){current.peerViewport?.takeIf{it.documentId==doc.documentId}?.let{peer->drawPath(polyline(peer.corners,true),Color(0xffecbb72),style=Stroke((1.0/current.view.camera.scale).toFloat(),pathEffect=PathEffect.dashPathEffect(floatArrayOf((6/current.view.camera.scale).toFloat(),(4/current.view.camera.scale).toFloat()))))}}
        }
    }
}
