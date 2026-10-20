package com.visualworkbench.desktop

import androidx.compose.ui.input.pointer.PointerType
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first

internal data class StreamContactSample(val point:Point,val time:Long,val pressure:Float=1f,
    val shift:Boolean=false,val semantic:Boolean=false)
internal fun streamContactToolAllowed(type:PointerType,primary:Boolean,secondary:Boolean,tertiary:Boolean):Boolean =
    !secondary&&!tertiary&&(type==PointerType.Stylus||(type==PointerType.Mouse&&primary))
private enum class StreamContactKind { Down, Move, Up }

/** One accepted real contact. While freeze/editor preparation suspends, retain
 * at most 4,096 real samples in order. Overflow or interruption cancels the
 * unfinished gesture; no prediction, replay onto another child, or retry. */
internal class DesktopStreamContactInput(
    private val scope:CoroutineScope,
    private val child:()->StreamDraftEditorAdapter?,
    private val admit:()->Boolean,
    private val freeze:suspend(Point,Camera)->StreamDraftEditorAdapter?,
    private val report:(String)->Unit,
) {
    private val pending=mutableListOf<Pair<StreamContactKind,StreamContactSample>>()
    private var work:Job?=null
    private var serial=0L
    private var held=false
    private var target:StreamDraftEditorAdapter?=null
    private var viewport:Triple<Double,Double,Double>?=null
    val busy:Boolean get()=held||work?.isCompleted==false

    fun viewport(width:Double,height:Double,density:Double){
        val next=Triple(width,height,density)
        if(viewport!=null&&viewport!=next)cancel()
        viewport=next
    }

    fun down(sample:StreamContactSample,camera:Camera,density:Double):Boolean {
        if(!admit()||busy||!density.isFinite()||density<=0)return false
        viewport(camera.viewportWidth,camera.viewportHeight,density)
        val geometry=viewport
        val editor=child()
        if(editor!=null){
            if(!editor.ready||!editor.state.value.canvasReady)return false
            held=true;target=editor;dispatch(editor,StreamContactKind.Down,sample);return true
        }
        held=true
        val ticket=++serial
        pending+=StreamContactKind.Down to sample
        val next=scope.launch(start=CoroutineStart.LAZY){
            var acquired:StreamDraftEditorAdapter?=null
            var replayed=false
            try{
                acquired=freeze(Point(sample.point.x/density,sample.point.y/density),camera)?:return@launch
                check(acquired.renderLifetime.register(checkNotNull(currentCoroutineContext()[Job])))
                acquired.state.first{it.canvasReady||it.renderIssue!=null||it.document==null}
                currentCoroutineContext().ensureActive()
                if(ticket!=serial||viewport!=geometry||!admit()||child()!==acquired||!acquired.state.value.canvasReady||
                    acquired.state.value.view.camera.viewportWidth!=camera.viewportWidth||
                    acquired.state.value.view.camera.viewportHeight!=camera.viewportHeight)return@launch
                target=acquired
                replayed=true
                val samples=pending.toList();pending.clear()
                for((kind,value)in samples)dispatch(acquired,kind,value)
            }catch(cancel:CancellationException){throw cancel}
            catch(_:Exception){report("The first annotation contact could not finish. The frozen draft remains owned.")}
            finally{
                if(ticket==serial){
                    pending.clear()
                    if(!replayed){held=false;acquired?.controller?.cancelInput()}
                }
            }
        }
        work=next
        next.invokeOnCompletion{if(work===next)work=null}
        next.start()
        return true
    }
    fun move(sample:StreamContactSample){if(held)offer(StreamContactKind.Move,sample)}
    fun up(sample:StreamContactSample){if(held){held=false;offer(StreamContactKind.Up,sample)}}
    private fun offer(kind:StreamContactKind,sample:StreamContactSample){
        val editor=target
        if(editor!=null){
            if(child()===editor&&admit())dispatch(editor,kind,sample)else cancel()
        }else{
            if(pending.size>=4096){cancel();report("This contact exceeded the annotation input limit. Draw a shorter stroke.")}
            else pending+=kind to sample
        }
    }
    private fun dispatch(editor:StreamDraftEditorAdapter,kind:StreamContactKind,sample:StreamContactSample){
        val controller=editor.controller
        when(kind){
            StreamContactKind.Down->controller.precisionDown(sample.point,sample.shift,false,sample.time,sample.pressure,sample.semantic)
            StreamContactKind.Move->controller.precisionMove(sample.point,sample.time,sample.pressure,sample.shift)
            StreamContactKind.Up->{controller.precisionUp(sample.point,sample.time,sample.pressure,sample.shift);controller.finishPointerContact();target=null}
        }
    }
    fun cancel(){
        serial++;held=false;pending.clear()
        work?.cancel()
        target?.controller?.cancelInput();target=null
    }
}

internal fun streamContactCamera(frame:CaptureStreamFrame,width:Double,height:Double):Camera {
    require(width.isFinite()&&height.isFinite()&&width>0&&height>0&&frame.width>0u&&frame.height>0u)
    return Camera(Point(frame.width.toDouble()/2,frame.height.toDouble()/2),
        minOf(width/frame.width.toDouble(),height/frame.height.toDouble()),0.0,width,height)
}

/** Compose coordinates are physical pixels; AWT's Graphics2D applies its own
 * device transform. Convert exactly once at this boundary. */
internal fun streamAwtCamera(camera:Camera,density:Double):Camera {
    require(density.isFinite()&&density>0)
    return camera.copy(scale=camera.scale/density,viewportWidth=camera.viewportWidth/density,
        viewportHeight=camera.viewportHeight/density)
}
