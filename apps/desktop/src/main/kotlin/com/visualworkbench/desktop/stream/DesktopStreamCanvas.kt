package com.visualworkbench.desktop.stream

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.awt.SwingPanel
import com.visualworkbench.shared.*
import kotlinx.coroutines.CompletableDeferred
import java.awt.BorderLayout
import java.awt.Color
import java.awt.Graphics
import java.awt.Graphics2D
import java.awt.geom.AffineTransform
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

/** AWT paint is synchronous. The bitmap is never released while its paint call
 * is active, and the previous acknowledged bitmap survives until native ACK. */
internal class DesktopStreamPainter(private val core:WorkbenchCore,private val wake:()->Unit,
    private val quarantine:(StreamRaster)->Unit,private val paintFailed:(Throwable)->Unit):DesktopStreamBackend {
    private var positive:StreamCanvasSlot?=null
    private var pending:StreamCanvasSlot?=null
    private var camera:Camera?=null
    private var sealed=false
    private val panel=object:JPanel() {
        override fun addNotify(){super.addNotify();wake()}
        override fun removeNotify(){pending?.permit?.refuse();pending?.losslessPaint?.cancel();super.removeNotify()}
        override fun paintComponent(graphics:Graphics){
            super.paintComponent(graphics)
            val trial=pending
            val slot=if(trial?.permit?.canDisplay()==false)positive else trial?:positive
            if(slot==null||sealed)return
            if(surfaceBytes()>slot.raster.admittedSurfaceBytes){
                val error=IllegalStateException("Capture drawing surface exceeded its reservation")
                trial?.permit?.refuse();trial?.losslessPaint?.completeExceptionally(error)
                paintFailed(error);return
            }
            try {
                val draw={drawSlot(graphics as Graphics2D,slot)}
                if(slot===trial&&trial.permit!=null)trial.permit.paint(draw) else draw()
                if(slot===trial)trial.losslessPaint?.complete(Unit)
            }catch(error:Throwable){
                trial?.losslessPaint?.completeExceptionally(error)
                paintFailed(error)
            }
        }
    }.apply { isOpaque=true;background=Color(12,17,24) }
    override val container:JPanel=JPanel(BorderLayout()).apply { isOpaque=true;background=Color(12,17,24);add(panel,BorderLayout.CENTER) }

    private fun checkEdt(){check(SwingUtilities.isEventDispatchThread())}
    private fun cameraFor(frame:CaptureStreamFrame):Camera {
        val width=panel.width.toDouble();val height=panel.height.toDouble()
        require(width>0.0&&height>0.0)
        return camera?.copy(viewportWidth=width,viewportHeight=height) ?: Camera(
            Point(frame.width.toDouble()/2.0,frame.height.toDouble()/2.0),
            minOf(width/frame.width.toDouble(),height/frame.height.toDouble()),0.0,width,height)
    }
    private fun drawSlot(graphics:Graphics2D,slot:StreamCanvasSlot){
        val value=checkNotNull(slot.raster.image)
        val transform=core.cameraMatrix(cameraFor(slot.frame))
        require(listOf(transform.a,transform.b,transform.c,transform.d,transform.e,transform.f).all(Double::isFinite))
        check(graphics.drawImage(value,AffineTransform(transform.a,transform.b,transform.c,transform.d,transform.e,transform.f),null))
    }
    override fun setCamera(value:Camera?){checkEdt();camera=value;panel.repaint()}
    override fun mapPointer(point:Point):Point? {
        checkEdt();val frame=positive?.frame?:return null
        return core.mapPoints(cameraFor(frame),true,listOf(point)).singleOrNull()
    }
    override fun surfaceBytes():Long {checkEdt();return panel.width.toLong()*panel.height.toLong()*4L}
    override fun offer(slot:StreamCanvasSlot):Boolean {
        checkEdt()
        if(!canStartDraw()||surfaceBytes()>slot.raster.admittedSurfaceBytes)return false
        pending=slot;panel.repaint();return true
    }
    override fun canStartDraw():Boolean {checkEdt();return !sealed&&pending==null&&panel.isShowing&&
        surfaceBytes() in 4..STREAM_SURFACE_RGBA_MAX.toLong()}
    override fun commit(slot:StreamCanvasSlot){
        checkEdt();check(pending===slot&&!sealed)
        val old=positive;positive=slot;pending=null
        try{old?.raster?.close()}catch(error:Throwable){old?.raster?.let(quarantine);sealed=true;throw error}
        panel.repaint()
    }
    override fun rollback(slot:StreamCanvasSlot,offered:Boolean){
        checkEdt()
        if(offered&&pending===slot)pending=null
        try{slot.raster.close()}catch(error:Throwable){quarantine(slot.raster);sealed=true;throw error}
        panel.repaint()
    }
    override fun retire(){
        checkEdt();sealed=true
        // Removing the painter on EDT joins any active synchronous paint before
        // releasing rasters; SwingPanel may keep the inert outer container.
        container.remove(panel);container.revalidate();container.repaint()
        val trial=pending;pending=null;trial?.permit?.refuse();trial?.losslessPaint?.cancel()
        val shown=positive;positive=null
        var failure:Throwable?=null
        for(slot in listOfNotNull(trial,shown))try{slot.raster.close()}catch(error:Throwable){quarantine(slot.raster);if(failure==null)failure=error else failure.addSuppressed(error)}
        failure?.let{throw it}
    }
}

/** Root must compose this canvas before admitting live frames. The same owner
 * supplies mapPointer() for exact full-frame physical D coordinates. */
@Composable internal fun DesktopStreamCanvas(view:DesktopStreamView,modifier:Modifier=Modifier){
    SwingPanel(factory={view.container()},modifier=modifier)
}
