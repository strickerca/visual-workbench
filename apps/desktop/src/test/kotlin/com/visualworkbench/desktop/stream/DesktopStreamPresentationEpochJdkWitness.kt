package com.visualworkbench.desktop.stream

import java.awt.AWTEvent
import java.awt.Color
import java.awt.Rectangle
import java.awt.event.InvocationEvent
import java.awt.event.PaintEvent
import java.awt.geom.AffineTransform
import java.awt.image.BufferedImage
import javax.swing.SwingUtilities
import kotlinx.coroutines.runBlocking

/** Projects the reviewed D3D ordering through the actual JDK native pending
 * event queue and actual SurfaceData lost/restored transitions, without GUI.
 * Requires --add-exports for java.desktop/sun.java2d and java.desktop/sun.awt. */
internal object DesktopStreamPresentationEpochJdkWitness {
    @JvmStatic fun main(args:Array<String>)=runBlocking {
        val appContext=Class.forName("sun.awt.AppContext")
        val getContext=appContext.getMethod("getAppContext")
        val postNative=Class.forName("sun.awt.SunToolkit").getMethod("postEvent",appContext,AWTEvent::class.java)
        val getSurface=Class.forName("sun.java2d.SunGraphics2D").getMethod("getSurfaceData")
        val setLost=Class.forName("sun.java2d.SurfaceData").getMethod("setSurfaceLost",Boolean::class.javaPrimitiveType)
        fun post(event:AWTEvent){postNative.invoke(null,getContext.invoke(null),event)}
        fun attempt(guarded:Boolean,replacement:Boolean=false):Boolean? {
            val permit=StreamDrawPermit()
            SwingUtilities.invokeAndWait {
                val canvas=BoundedStreamCanvas({AffineTransform()},{})
                val operation=if(guarded)canvas.beginDestinationSubmission() else StreamDestinationState{true}
                val image=BufferedImage(2,2,BufferedImage.TYPE_INT_RGB)
                val graphics=image.createGraphics()
                try {
                    runCatching{permit.paint{submitStreamDestination(graphics,operation,{
                        graphics.color=Color.RED;graphics.fillRect(0,0,2,2)
                        val surface=getSurface.invoke(graphics)
                        setLost.invoke(surface,true);setLost.invoke(surface,false)
                        graphics.color=Color.BLACK;graphics.fillRect(0,0,2,2)
                        // The updater runs inside drawImage, clears the frame,
                        // and posts a request before the caller gets a tracker.
                        if(replacement)post(InvocationEvent(canvas,Runnable{}))
                        else post(PaintEvent(canvas,PaintEvent.PAINT,Rectangle(0,0,2,2)))
                        true
                    },{})}}
                }finally{graphics.dispose();image.flush()}
            }
            return runBlocking{permit.awaitDecision()}
        }
        check(attempt(false)==true){"Negative control did not reproduce the late-tracker defect"}
        check(attempt(true)==null){"Native pending repaint accepted a positive receipt"}
        check(attempt(true,true)==null){"Native pending replacement accepted a positive receipt"}
        val normal=StreamDrawPermit()
        SwingUtilities.invokeAndWait {
            val canvas=BoundedStreamCanvas({AffineTransform()},{})
            // Preparation may restore an empty destination before frame writes.
            post(PaintEvent(canvas,PaintEvent.PAINT,Rectangle(0,0,2,2)))
            val operation=canvas.beginDestinationSubmission()
            val image=BufferedImage(2,2,BufferedImage.TYPE_INT_RGB)
            val graphics=image.createGraphics()
            try{normal.paint{submitStreamDestination(graphics,operation,{
                graphics.color=Color.RED;graphics.fillRect(0,0,2,2);true
            },{})}}finally{graphics.dispose();image.flush()}
        }
        check(normal.awaitDecision()==true)
        println("JDK whole-operation witness passed: late-tracker control=true; in-draw native repaint/replacement=null; normal=true")
    }
}
