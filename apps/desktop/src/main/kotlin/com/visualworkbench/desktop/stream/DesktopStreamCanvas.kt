package com.visualworkbench.desktop.stream

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.awt.SwingPanel
import com.visualworkbench.shared.*
import kotlinx.coroutines.CompletableDeferred
import java.awt.BorderLayout
import java.awt.Canvas
import java.awt.Color
import java.awt.Graphics
import java.awt.Graphics2D
import java.awt.Toolkit
import java.awt.geom.AffineTransform
import java.awt.image.BufferStrategy
import kotlin.math.ceil
import javax.swing.JPanel
import javax.swing.SwingUtilities

internal data class StreamCanvasSlot(val frame:CaptureStreamFrame,val raster:StreamRaster,
    val permit:StreamDrawPermit?=null,val losslessPaint:CompletableDeferred<Unit>?=null)

internal interface DesktopStreamBackend {
    val container:JPanel
    fun setCamera(value:Camera?)
    fun mapPointer(point:Point):Point?
    fun surfaceBytes():Long
    fun canStartDraw():Boolean
    fun offer(slot:StreamCanvasSlot):Boolean
    fun commit(slot:StreamCanvasSlot)
    fun rollback(slot:StreamCanvasSlot,offered:Boolean)
    fun retire()
}

/** Physical destination size, before allocating or receiving source pixels. */
internal fun physicalCanvasBytes(width:Int,height:Int,scaleX:Double,scaleY:Double):Long {
    require(width>0&&height>0&&scaleX.isFinite()&&scaleY.isFinite()&&scaleX>0&&scaleY>0)
    val w=ceil(width.toDouble()*scaleX)
    val h=ceil(height.toDouble()*scaleY)
    require(w<=STREAM_SURFACE_RGBA_MAX.toDouble()/4.0&&h<=STREAM_SURFACE_RGBA_MAX.toDouble()/4.0)
    val bytes=w.toLong()*h.toLong()*4L
    require(bytes in 4..STREAM_SURFACE_RGBA_MAX.toLong())
    return bytes
}

/** A successful return means the strategy accepted a visible-buffer show. */
internal fun submitStreamBuffer(strategy:BufferStrategy,draw:(Graphics2D)->Unit,afterShow:()->Unit){
    var attempts=0
    do {
        check(++attempts<=3)
        val graphics=strategy.drawGraphics
        try{draw(graphics as Graphics2D)}finally{graphics.dispose()}
    }while(strategy.contentsRestored())
    check(!strategy.contentsLost())
    strategy.show()
    afterShow()
    check(!strategy.contentsLost())
}

/** The receipt follows BufferStrategy.show(), which publishes the drawn back
 * buffer to this Canvas. Swing paint callbacks and queued D3D window updates
 * are not presentation receipts. This is no optical scanout timestamp. */
internal class DesktopStreamPainter(private val core:WorkbenchCore,private val wake:()->Unit,
    private val quarantine:(StreamRaster)->Unit,private val paintFailed:(Throwable)->Unit):DesktopStreamBackend {
    private var positive:StreamCanvasSlot?=null
    private var pending:StreamCanvasSlot?=null
    private var camera:Camera?=null
    private var sealed=false
    private var strategySurfaceBytes=0L
    private val retained=mutableListOf<StreamRaster>()
    private val canvas=object:Canvas() {
        override fun addNotify(){super.addNotify();wake()}
        override fun removeNotify(){pending?.permit?.refuse();pending?.losslessPaint?.cancel();super.removeNotify()}
        override fun paint(graphics:Graphics){
            val trial=pending
            val slot=if(trial?.permit?.canDisplay()==false)positive else trial?:positive
            if(slot==null)return
            if(sealed)return
            try {
                check(surfaceBytes()<=slot.raster.admittedSurfaceBytes)
                val strategy=bufferStrategy?:return
                show(slot,strategy)
            }catch(error:Throwable){paintFailed(error)}
        }
    }.apply {background=Color(12,17,24)}
    override val container:JPanel=JPanel(BorderLayout()).apply {
        isOpaque=true;isDoubleBuffered=false;background=Color(12,17,24);add(canvas,BorderLayout.CENTER)
    }

    private fun checkEdt(){check(SwingUtilities.isEventDispatchThread())}
    private fun cameraFor(frame:CaptureStreamFrame):Camera {
        val width=canvas.width.toDouble();val height=canvas.height.toDouble()
        require(width>0.0&&height>0.0)
        return camera?.copy(viewportWidth=width,viewportHeight=height) ?: Camera(
            Point(frame.width.toDouble()/2.0,frame.height.toDouble()/2.0),
            minOf(width/frame.width.toDouble(),height/frame.height.toDouble()),0.0,width,height)
    }
    private fun drawSlot(graphics:Graphics2D,slot:StreamCanvasSlot){
        val value=checkNotNull(slot.raster.image)
        val transform=core.cameraMatrix(cameraFor(slot.frame))
        require(listOf(transform.a,transform.b,transform.c,transform.d,transform.e,transform.f).all(Double::isFinite))
        graphics.color=canvas.background
        graphics.fillRect(0,0,canvas.width,canvas.height)
        check(graphics.drawImage(value,AffineTransform(transform.a,transform.b,transform.c,transform.d,transform.e,transform.f),null))
    }
    private fun strategy():BufferStrategy {
        check(canvas.isShowing)
        strategySurfaceBytes=maxOf(strategySurfaceBytes,currentSurfaceBytes())
        if(canvas.bufferStrategy==null)canvas.createBufferStrategy(2)
        return checkNotNull(canvas.bufferStrategy)
    }
    private fun show(slot:StreamCanvasSlot,strategy:BufferStrategy){
        checkEdt()
        submitStreamBuffer(strategy,{drawSlot(it,slot)},{
            Toolkit.getDefaultToolkit().sync()
            check(canvas.isShowing&&surfaceBytes()<=slot.raster.admittedSurfaceBytes)
        })
    }
    override fun setCamera(value:Camera?){checkEdt();camera=value;canvas.repaint()}
    override fun mapPointer(point:Point):Point? {
        checkEdt();val frame=positive?.frame?:return null
        return core.mapPoints(cameraFor(frame),true,listOf(point)).singleOrNull()
    }
    private fun currentSurfaceBytes():Long {
        checkEdt()
        val scale=canvas.graphicsConfiguration?.defaultTransform
        return physicalCanvasBytes(canvas.width,canvas.height,scale?.scaleX?:1.0,scale?.scaleY?:1.0)
    }
    override fun surfaceBytes():Long=maxOf(currentSurfaceBytes(),strategySurfaceBytes)
    override fun offer(slot:StreamCanvasSlot):Boolean {
        checkEdt()
        if(!canStartDraw()||surfaceBytes()>slot.raster.admittedSurfaceBytes)return false
        pending=slot
        try {
            val strategy=strategy()
            val submit={show(slot,strategy)}
            if(slot.permit!=null)slot.permit.paint(submit) else submit()
            slot.losslessPaint?.complete(Unit)
            return true
        }catch(error:Throwable){
            slot.losslessPaint?.completeExceptionally(error)
            paintFailed(error)
            throw error
        }
    }
    override fun canStartDraw():Boolean {
        checkEdt()
        return !sealed&&pending==null&&canvas.isShowing&&
            runCatching{surfaceBytes() in 4..STREAM_SURFACE_RGBA_MAX.toLong()}.getOrDefault(false)
    }
    override fun commit(slot:StreamCanvasSlot){
        checkEdt();check(pending===slot&&!sealed)
        val old=positive;positive=slot;pending=null
        try{old?.raster?.close()}catch(error:Throwable){old?.raster?.let(quarantine);sealed=true;throw error}
    }
    override fun rollback(slot:StreamCanvasSlot,offered:Boolean){
        checkEdt()
        val installed=pending===slot
        if(installed)pending=null
        var failure:Throwable?=null
        if(installed)try {
            val previous=positive
            if(previous!=null&&canvas.isShowing){
                check(surfaceBytes()<=previous.raster.admittedSurfaceBytes)
                show(previous,strategy())
            }else {canvas.bufferStrategy?.dispose();strategySurfaceBytes=0L;canvas.repaint()}
        }catch(error:Throwable){sealed=true;failure=error}
        if(failure!=null){retained+=slot.raster;throw failure}
        try{slot.raster.close()}catch(error:Throwable){
            quarantine(slot.raster);sealed=true
            failure=error
        }
        failure?.let{throw it}
    }
    override fun retire(){
        checkEdt();sealed=true
        val trial=pending;pending=null;trial?.permit?.refuse();trial?.losslessPaint?.cancel()
        val shown=positive;positive=null
        var failure:Throwable?=null
        try{canvas.bufferStrategy?.dispose();strategySurfaceBytes=0L}catch(error:Throwable){failure=error}
        try{container.remove(canvas);container.revalidate();container.repaint()}catch(error:Throwable){
            if(failure==null)failure=error else failure.addSuppressed(error)
        }
        if(failure!=null){
            listOfNotNull(trial,shown).forEach{quarantine(it.raster)}
            retained.forEach(quarantine)
            retained.clear()
            throw failure
        }
        for(raster in listOfNotNull(trial?.raster,shown?.raster)+retained)try{raster.close()}catch(error:Throwable){
            quarantine(raster);if(failure==null)failure=error else failure.addSuppressed(error)
        }
        retained.clear()
        failure?.let{throw it}
    }
}

@Composable internal fun DesktopStreamCanvas(view:DesktopStreamView,modifier:Modifier=Modifier){
    SwingPanel(factory={view.container()},modifier=modifier)
}
