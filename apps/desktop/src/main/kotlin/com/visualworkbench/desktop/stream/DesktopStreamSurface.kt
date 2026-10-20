package com.visualworkbench.desktop.stream

import java.awt.AWTException
import java.awt.BufferCapabilities
import java.awt.Canvas
import java.awt.Dimension
import java.awt.GraphicsConfiguration
import java.awt.Graphics
import java.awt.ImageCapabilities
import java.awt.Rectangle
import java.awt.Transparency
import java.awt.geom.AffineTransform
import java.awt.image.BufferedImage
import java.awt.image.BufferStrategy
import java.awt.image.ColorModel
import java.awt.image.VolatileImage

/** A retained software image must not follow a component's future DPI. The
 * public GraphicsConfiguration factory creates it without a Component owner;
 * this immutable configuration also checks every physical fallback allocation. */
internal class StreamImageConfiguration(private val source:GraphicsConfiguration,
    transform:AffineTransform,
    private val allocate:(Int,Int,Int)->BufferedImage={w,h,type->BufferedImage(w,h,type)}):GraphicsConfiguration() {
    private val scale=AffineTransform(transform)
    init {
        require(scale.shearX==0.0&&scale.shearY==0.0)
        physicalCanvasBytes(1,1,scale.scaleX,scale.scaleY)
    }
    override fun getDevice()=source.device
    override fun getColorModel():ColorModel=ColorModel.getRGBdefault()
    override fun getColorModel(transparency:Int):ColorModel?=when(transparency){
        Transparency.OPAQUE,Transparency.BITMASK,Transparency.TRANSLUCENT->colorModel
        else->null
    }
    override fun getDefaultTransform()=AffineTransform(scale)
    override fun getNormalizingTransform()=AffineTransform()
    override fun getBounds()=Rectangle(source.bounds)
    override fun createCompatibleImage(width:Int,height:Int):BufferedImage=
        createCompatibleImage(width,height,Transparency.OPAQUE)
    override fun createCompatibleImage(width:Int,height:Int,transparency:Int):BufferedImage {
        physicalCanvasBytes(width,height,1.0,1.0)
        require(transparency==Transparency.OPAQUE||transparency==Transparency.BITMASK||transparency==Transparency.TRANSLUCENT)
        return allocate(width,height,if(transparency==Transparency.OPAQUE)BufferedImage.TYPE_INT_RGB else BufferedImage.TYPE_INT_ARGB)
    }
}

/** Bound every dimension used by peer and software buffer factories. Own the
 * JDK blit strategy explicitly: Component's general factory can silently choose
 * a flip strategy whose component-owned images revalidate at a later DPI. */
internal open class BoundedStreamCanvas(private val peerTransform:(Canvas)->AffineTransform,
    private val reject:()->Unit,
    private val imageConfiguration:(Canvas)->GraphicsConfiguration?={it.graphicsConfiguration},
    private val allocate:(Int,Int,Int)->BufferedImage={w,h,type->BufferedImage(w,h,type)}):Canvas() {
    private var ownedStrategy:BufferStrategy?=null
    private fun bounded():Rectangle {
        val actual=Rectangle(super.getX(),super.getY(),super.getWidth(),super.getHeight())
        val scale=peerTransform(this)
        val value=boundedStreamPeerBounds(actual,scale.scaleX,scale.scaleY)
        if(value.width!=actual.width||value.height!=actual.height)reject()
        return value
    }
    override fun getBounds()=bounded()
    override fun getBounds(value:Rectangle?):Rectangle=(value?:Rectangle()).also{it.setBounds(bounded())}
    override fun getWidth()=bounded().width
    override fun getHeight()=bounded().height
    override fun getSize()=bounded().size
    override fun getSize(value:Dimension?):Dimension=(value?:Dimension()).also{it.setSize(bounded().size)}
    @Suppress("DEPRECATION", "OVERRIDE_DEPRECATION")
    override fun bounds()=bounded()
    @Suppress("DEPRECATION", "OVERRIDE_DEPRECATION")
    override fun size()=bounded().size
    override fun setBounds(x:Int,y:Int,width:Int,height:Int){
        val scale=peerTransform(this)
        if(width>0&&height>0&&runCatching{physicalCanvasBytes(width,height,scale.scaleX,scale.scaleY)}.isFailure){
            super.setBounds(x,y,0,0);reject();return
        }
        super.setBounds(x,y,width,height)
    }
    override fun createVolatileImage(width:Int,height:Int):VolatileImage {
        val transform=peerTransform(this)
        physicalCanvasBytes(width,height,transform.scaleX,transform.scaleY)
        val configuration=checkNotNull(imageConfiguration(this))
        return StreamImageConfiguration(configuration,transform,allocate)
            .createCompatibleVolatileImage(width,height,ImageCapabilities(false),Transparency.OPAQUE)
    }
    override fun createVolatileImage(width:Int,height:Int,caps:ImageCapabilities?):VolatileImage {
        if(caps?.isAccelerated==true)throw AWTException("Stream buffers require bounded software backing")
        return createVolatileImage(width,height)
    }
    override fun createBufferStrategy(numBuffers:Int){
        createBufferStrategy(numBuffers,BufferCapabilities(ImageCapabilities(false),ImageCapabilities(false),null))
    }
    override fun createBufferStrategy(numBuffers:Int,caps:BufferCapabilities){
        require(numBuffers==2&&!caps.isPageFlipping&&!caps.frontBufferCapabilities.isAccelerated&&!caps.backBufferCapabilities.isAccelerated)
        ownedStrategy?.dispose()
        ownedStrategy=StreamBltStrategy(numBuffers,caps)
    }
    override fun getBufferStrategy():BufferStrategy?=ownedStrategy
    private inner class StreamBltStrategy(numBuffers:Int,caps:BufferCapabilities):BltBufferStrategy(numBuffers,caps) {
        private var creationTransform=AffineTransform(peerTransform(this@BoundedStreamCanvas))
        override fun getDrawGraphics():Graphics {
            val current=peerTransform(this@BoundedStreamCanvas)
            if(current!=creationTransform){
                // The old image retains its immutable scale. Replace it through
                // the checked factory before drawing at the new scale instead
                // of allowing its surface manager to expand it autonomously.
                createBackBuffers(1)
                creationTransform=AffineTransform(current)
            }
            return super.getDrawGraphics()
        }
        override fun dispose(){super.dispose();if(ownedStrategy===this)ownedStrategy=null}
    }
}
