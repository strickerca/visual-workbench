package com.visualworkbench.desktop.stream

import java.awt.Canvas
import java.awt.GraphicsConfiguration
import java.awt.Image
import java.awt.ImageCapabilities
import java.awt.Transparency
import java.awt.geom.AffineTransform
import java.awt.image.BufferedImage
import java.awt.image.VolatileImage

/** Explicit focused JDK check; no GUI/device. Run with
 * --add-exports=java.desktop/sun.awt.image=ALL-UNNAMED to invoke the actual
 * display-change callback. Allocation is intercepted before creating arrays. */
internal object DesktopStreamSurfaceJdkWitness {
    @JvmStatic fun main(args:Array<String>){
        val seed=BufferedImage(1,1,BufferedImage.TYPE_INT_RGB)
        val source=seed.createGraphics().let{try{it.deviceConfiguration}finally{it.dispose()}}
        var dpi=1.0
        val requests=mutableListOf<Long>()
        val mutableConfiguration=object:GraphicsConfiguration(){
            override fun getDevice()=source.device
            override fun getColorModel()=source.colorModel
            override fun getColorModel(transparency:Int)=source.getColorModel(transparency)
            override fun getDefaultTransform()=AffineTransform.getScaleInstance(dpi,dpi)
            override fun getNormalizingTransform()=AffineTransform()
            override fun getBounds()=source.bounds
            override fun createCompatibleImage(width:Int,height:Int,transparency:Int):BufferedImage {
                requests+=width.toLong()*height.toLong()*4L
                return BufferedImage(1,1,BufferedImage.TYPE_INT_RGB)
            }
        }
        val managerClass=Class.forName("sun.awt.image.SurfaceManager")
        val getManager=managerClass.getMethod("getManager",Image::class.java)
        val displayChanged=Class.forName("sun.awt.image.VolatileSurfaceManager").getMethod("displayChanged")
        fun change(image:VolatileImage){displayChanged.invoke(getManager.invoke(null,image))}

        val control=mutableConfiguration.createCompatibleVolatileImage(2896,2896,ImageCapabilities(false),Transparency.OPAQUE)
        dpi=3.0
        change(control)
        val controlBytes=requests.last()
        check(controlBytes==301_925_376L&&controlBytes>STREAM_PLATFORM_SURFACE_RESERVE)
        control.flush();requests.clear();dpi=1.0

        val retained=mutableListOf<VolatileImage>()
        val canvas=object:BoundedStreamCanvas(
            {_:Canvas->mutableConfiguration.defaultTransform},{}, {mutableConfiguration},
            {width,height,type->requests+=width.toLong()*height.toLong()*4L;BufferedImage(1,1,type)}){
            override fun createVolatileImage(width:Int,height:Int):VolatileImage=
                super.createVolatileImage(width,height).also(retained::add)
        }
        canvas.setBounds(0,0,2896,2896)
        canvas.createBufferStrategy(2)
        try {
            val old=retained.single()
            val before=requests.size
            dpi=3.0
            change(old) // retained image must not adopt the new component DPI
            check(requests.size==before)
            old.flush();old.validate(mutableConfiguration);old.snapshot.flush()
            canvas.bufferStrategy!!.drawGraphics.dispose() // real JDK blit revalidation/new bounded image
            check(retained.size>=2)
            check(requests.all{it in 4..STREAM_SURFACE_RGBA_MAX.toLong()})
            println("JDK display-change witness passed: control=$controlBytes bytes; all production allocations <= ${STREAM_SURFACE_RGBA_MAX} bytes")
        }finally{canvas.bufferStrategy?.dispose();seed.flush()}
    }
}
