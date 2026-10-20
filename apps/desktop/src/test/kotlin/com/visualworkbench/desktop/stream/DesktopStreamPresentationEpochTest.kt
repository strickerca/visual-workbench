package com.visualworkbench.desktop.stream

import java.awt.Rectangle
import java.awt.Toolkit
import java.awt.event.InvocationEvent
import java.awt.event.PaintEvent
import java.awt.geom.AffineTransform
import java.awt.image.BufferedImage
import javax.swing.SwingUtilities
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class DesktopStreamPresentationEpochTest {
    private enum class Boundary { BEFORE_OBSERVATION, BEFORE_DRAW, INSIDE_DRAW, TRACKER_ACQUISITION, SYNC, FINAL_OBSERVATION, SUBMISSION_CHECK }
    private class Surface:StreamDestinationSurface {
        override val identity=Any()
        var generation=0
        var dirty=false
        var onState:()->Unit={}
        var onSubmitted:()->Unit={}
        override fun isValid()=true
        override fun isLost()=false // model restoration already clearing lost
        override fun isSubmitted():Boolean {onSubmitted();return !dirty}
        override fun state():StreamDestinationState {
            onState()
            val captured=generation
            return StreamDestinationState{captured==generation}
        }
    }

    @Test fun restoredDestinationIsUnknownAtEveryDrawAndSubmitBoundary()=runBlocking {
        for(boundary in Boundary.entries){
            val permit=StreamDrawPermit()
            SwingUtilities.invokeAndWait {
                val canvas=BoundedStreamCanvas({AffineTransform()},{})
                val operation=canvas.beginDestinationSubmission()
                val surface=Surface()
                val queue=Toolkit.getDefaultToolkit().systemEventQueue
                fun restored(){
                    surface.generation++;surface.dirty=false
                    queue.postEvent(PaintEvent(canvas,PaintEvent.PAINT,Rectangle(0,0,2,2)))
                }
                if(boundary==Boundary.BEFORE_OBSERVATION)restored()
                if(boundary==Boundary.TRACKER_ACQUISITION)surface.onState=::restored
                if(boundary==Boundary.SUBMISSION_CHECK)surface.onSubmitted=::restored
                var observations=0
                val graphics=BufferedImage(2,2,BufferedImage.TYPE_INT_RGB).createGraphics()
                try {
                    val result=runCatching{permit.paint{submitStreamDestination(graphics,operation,
                        {surface.generation++;surface.dirty=true;if(boundary==Boundary.INSIDE_DRAW)restored();true},
                        {surface.dirty=false;if(boundary==Boundary.SYNC)restored()},
                        {
                            observations++
                            if(boundary==Boundary.BEFORE_DRAW&&observations==1)restored()
                            if(boundary==Boundary.FINAL_OBSERVATION&&observations==3)restored()
                            surface
                        })}}
                    check(result.isFailure){"Restoration at $boundary returned positive"}
                }finally{graphics.dispose()}
            }
            assertNull(permit.awaitDecision())
        }
    }

    @Test fun updaterReplacementQueuedInsideDrawCannotBecomeAPositiveReceipt()=runBlocking {
        val permit=StreamDrawPermit()
        SwingUtilities.invokeAndWait {
            val canvas=BoundedStreamCanvas({AffineTransform()},{})
            val operation=canvas.beginDestinationSubmission()
            val surface=Surface()
            val graphics=BufferedImage(2,2,BufferedImage.TYPE_INT_RGB).createGraphics()
            try {
                assertThrows(IllegalStateException::class.java){permit.paint{submitStreamDestination(graphics,operation,
                    {
                        Toolkit.getDefaultToolkit().systemEventQueue.postEvent(InvocationEvent(canvas,Runnable{}))
                        surface.generation++;surface.dirty=false;true
                    },{},{surface})}}
            }finally{graphics.dispose()}
        }
        assertNull(permit.awaitDecision())
    }

    @Test fun normalImplicitUpdaterInsideDrawAndExplicitSyncBothRemainUsable()=runBlocking {
        for(implicitUpdater in listOf(false,true)){
            val permit=StreamDrawPermit()
            SwingUtilities.invokeAndWait {
                val canvas=BoundedStreamCanvas({AffineTransform()},{})
                val operation=canvas.beginDestinationSubmission()
                val surface=Surface()
                val graphics=BufferedImage(2,2,BufferedImage.TYPE_INT_RGB).createGraphics()
                try {
                    assertTrue(permit.paint{submitStreamDestination(graphics,operation,
                        {surface.generation++;surface.dirty=!implicitUpdater;true},
                        {surface.dirty=false},{surface})})
                }finally{graphics.dispose()}
            }
            assertEquals(true,permit.awaitDecision())
        }
    }

    @Test fun emptyDestinationRestoredBeforeAnyFrameWriteCanEstablishANormalBaseline()=runBlocking {
        val permit=StreamDrawPermit()
        SwingUtilities.invokeAndWait {
            val canvas=BoundedStreamCanvas({AffineTransform()},{})
            Toolkit.getDefaultToolkit().systemEventQueue.postEvent(PaintEvent(canvas,PaintEvent.PAINT,Rectangle(0,0,2,2)))
            val operation=canvas.beginDestinationSubmission()
            val surface=Surface()
            val graphics=BufferedImage(2,2,BufferedImage.TYPE_INT_RGB).createGraphics()
            try {
                assertTrue(permit.paint{submitStreamDestination(graphics,operation,
                    {surface.generation++;surface.dirty=true;true},{surface.dirty=false},{surface})})
            }finally{graphics.dispose()}
        }
        assertEquals(true,permit.awaitDecision())
    }
}
