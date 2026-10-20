package com.visualworkbench.desktop.stream

import java.awt.BufferCapabilities
import java.awt.Graphics
import java.awt.ImageCapabilities
import java.awt.image.BufferedImage
import java.awt.image.BufferStrategy
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class DesktopStreamDestinationTest {
    private class Surface:StreamDestinationSurface {
        override val identity=Any()
        var valid=true
        var lost=false
        var dirty=false
        var generation=0
        override fun isValid()=valid
        override fun isLost()=lost
        override fun isSubmitted()=!dirty
        override fun state():StreamDestinationState {
            val captured=generation
            return StreamDestinationState{generation==captured}
        }
    }
    private fun strategy(surface:Surface,sync:()->Unit)=object:BufferStrategy(){
        override fun getCapabilities()=BufferCapabilities(ImageCapabilities(false),ImageCapabilities(false),null)
        override fun getDrawGraphics():Graphics=BufferedImage(2,2,BufferedImage.TYPE_INT_RGB).createGraphics()
        override fun contentsLost()=false // software source survives destination loss
        override fun contentsRestored()=false
        override fun show(){
            val graphics=BufferedImage(2,2,BufferedImage.TYPE_INT_RGB).createGraphics()
            try{submitStreamDestination(graphics,StreamDestinationState{true},{surface.dirty=true;true},sync,{surface})}
            finally{graphics.dispose()}
        }
    }

    @Test fun normalDestinationSubmissionCompletesAPositiveReceipt()=runBlocking {
        val destination=Surface()
        val buffer=strategy(destination){destination.dirty=false}
        val permit=StreamDrawPermit()
        assertTrue(permit.paint{submitStreamBuffer(buffer,{},{})})
        assertEquals(true,permit.awaitDecision())
        assertFalse(buffer.contentsLost())
    }

    @Test fun destinationLossRestoredOnlyForRepaintStaysUnknownEvenWhenSourceSurvives()=runBlocking {
        val destination=Surface()
        val buffer=strategy(destination){
            destination.lost=true;destination.generation++
            destination.lost=false;destination.dirty=false // D3D restores, clears and queues repaint
        }
        val permit=StreamDrawPermit()
        assertThrows(IllegalStateException::class.java){permit.paint{submitStreamBuffer(buffer,{},{})}}
        assertNull(permit.awaitDecision())
        assertFalse(buffer.contentsLost())
        assertFalse(destination.lost)
        assertFalse(destination.dirty)
    }

    @Test fun stillDirtyDestinationCannotClaimSubmission()=runBlocking {
        val destination=Surface()
        val permit=StreamDrawPermit()
        assertThrows(IllegalStateException::class.java){permit.paint{submitStreamBuffer(strategy(destination){},{},{})}}
        assertNull(permit.awaitDecision())
    }

    @Test fun repaintOnlyDestinationStateCannotClaimTheOriginalDraw()=runBlocking {
        val destination=Surface()
        val buffer=strategy(destination){destination.generation++;destination.dirty=false}
        val permit=StreamDrawPermit()
        assertThrows(IllegalStateException::class.java){permit.paint{submitStreamBuffer(buffer,{},{})}}
        assertNull(permit.awaitDecision())
    }
}
