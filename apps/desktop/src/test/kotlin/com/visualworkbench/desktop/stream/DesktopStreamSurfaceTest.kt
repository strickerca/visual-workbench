package com.visualworkbench.desktop.stream

import java.awt.Canvas
import java.awt.ImageCapabilities
import java.awt.geom.AffineTransform
import java.awt.image.BufferedImage
import java.awt.image.VolatileImage
import org.junit.Assert.*
import org.junit.Test

class DesktopStreamSurfaceTest {
    private class Witness {
        var dpi=1.0
        var rejected=0
        val allocations=mutableListOf<Pair<Int,Int>>()
        val images=mutableListOf<VolatileImage>()
        private val seed=BufferedImage(1,1,BufferedImage.TYPE_INT_RGB)
        private val configuration=seed.createGraphics().let{try{it.deviceConfiguration}finally{it.dispose()}}
        val canvas=object:BoundedStreamCanvas(
            {_:Canvas->AffineTransform.getScaleInstance(dpi,dpi)}, {rejected++}, {configuration},
            {width,height,type->
                allocations+=width to height
                // Intercept the actual JDK software allocation before reserving
                // large arrays. All factory/validate paths remain real JDK code.
                BufferedImage(1,1,type)
            }) {
            override fun createVolatileImage(width:Int,height:Int):VolatileImage=
                super.createVolatileImage(width,height).also(images::add)
        }
        fun createInitialStrategy(){canvas.setBounds(0,0,2896,2896);canvas.createBufferStrategy(2)}
        fun assertBounded(){
            assertTrue(allocations.isNotEmpty())
            assertTrue(allocations.all{(width,height)->width.toLong()*height.toLong()*4L<=STREAM_SURFACE_RGBA_MAX})
        }
    }

    @Test fun actualJdkSoftwareFallbackAtThreeXNeverRequestsTheOldUnboundedDimensions() {
        val witness=Witness()
        witness.createInitialStrategy()
        try {
            assertEquals(301_925_376L,2896L*2896L*3L*3L*4L)
            assertTrue(301_925_376L>STREAM_PLATFORM_SURFACE_RESERVE)
            witness.dpi=3.0 // no setBounds and no queued rejection callback
            val bounds=witness.canvas.bounds
            assertEquals(bounds.width,witness.canvas.width)
            assertEquals(bounds.height,witness.canvas.height)
            assertEquals(bounds.size,witness.canvas.size)
            witness.canvas.bufferStrategy!!.drawGraphics.dispose() // JDK BltBufferStrategy.revalidate
            assertTrue(witness.images.size>=2)
            assertFalse(witness.canvas.bufferStrategy!!.capabilities.isPageFlipping)
            assertFalse(witness.canvas.bufferStrategy!!.capabilities.backBufferCapabilities.isAccelerated)
            assertTrue(witness.rejected>0)
            witness.assertBounded()
        }finally{witness.canvas.bufferStrategy?.dispose()}
    }

    @Test fun retainedSoftwareImageRevalidationAndSnapshotKeepTheirCreationScale() {
        val witness=Witness()
        witness.createInitialStrategy()
        try {
            val retained=witness.images.single()
            val creationDimensions=witness.allocations.single()
            witness.dpi=3.0
            retained.flush()
            retained.validate(null)
            retained.snapshot.flush()
            assertEquals(creationDimensions,witness.allocations.last())
            witness.assertBounded()
        }finally{witness.canvas.bufferStrategy?.dispose()}
    }

    @Test fun explicitStaleBackingDimensionsAreRejectedBeforeJdkAllocation() {
        val witness=Witness()
        witness.createInitialStrategy()
        try {
            witness.dpi=3.0
            val before=witness.allocations.size
            assertThrows(IllegalArgumentException::class.java){witness.canvas.createVolatileImage(2896,2896)}
            assertThrows(IllegalArgumentException::class.java){
                witness.canvas.createVolatileImage(2896,2896,ImageCapabilities(false))
            }
            assertEquals(before,witness.allocations.size)
            witness.assertBounded()
        }finally{witness.canvas.bufferStrategy?.dispose()}
    }

    @Test fun sameLogicalSizeGetsCheckedNewBackingWhileTheRetainedImageKeepsItsOldScale() {
        val witness=Witness()
        witness.canvas.setBounds(0,0,100,50)
        witness.canvas.createBufferStrategy(2)
        try {
            val retained=witness.images.single()
            witness.dpi=3.0
            witness.canvas.bufferStrategy!!.drawGraphics.dispose()
            assertEquals(300 to 150,witness.allocations.last())
            retained.validate(null)
            retained.snapshot.flush()
            assertEquals(100 to 50,witness.allocations.last())
            witness.assertBounded()
        }finally{witness.canvas.bufferStrategy?.dispose()}
    }
}
