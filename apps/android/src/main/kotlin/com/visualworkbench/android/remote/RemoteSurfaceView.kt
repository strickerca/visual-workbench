package com.visualworkbench.android.remote

import android.content.Context
import android.graphics.Color
import android.view.Gravity
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.widget.FrameLayout
import com.visualworkbench.shared.RemoteVideoConfig
import com.visualworkbench.shared.RemoteViewport
import kotlin.math.roundToInt

/** The visible viewport clips even-coded padding instead of stretching it.
 * Holder.surface is borrowed; the decoder duplicates it and owns only its copy. */
internal class RemoteSurfaceView(context:Context,private val controller:RemoteEditController):FrameLayout(context) {
    private val clip=FrameLayout(context)
    private val video=SurfaceView(context)
    private var configuration:RemoteVideoConfig?=null
    init {
        setBackgroundColor(Color.BLACK);clip.clipChildren=true;clip.clipToPadding=true
        clip.addView(video,LayoutParams(1,1));addView(clip,LayoutParams(1,1))
        video.holder.addCallback(object:SurfaceHolder.Callback {
            override fun surfaceCreated(holder:SurfaceHolder){controller.surface(holder.surface,this@RemoteSurfaceView)}
            override fun surfaceChanged(holder:SurfaceHolder,format:Int,width:Int,height:Int){controller.surface(holder.surface,this@RemoteSurfaceView)}
            override fun surfaceDestroyed(holder:SurfaceHolder){controller.surface(null,this@RemoteSurfaceView)}
        })
        isFocusable=true
    }
    fun configure(value:RemoteVideoConfig?){if(value==configuration)return;configuration=value;layoutVideo()}
    private fun layoutVideo(){
        val config=configuration?:return
        val viewport=RemoteViewport.fit(width,height,config.visibleWidth,config.visibleHeight)?:return
        clip.layoutParams=LayoutParams(viewport.width.roundToInt(),viewport.height.roundToInt()).apply{leftMargin=viewport.left.roundToInt();topMargin=viewport.top.roundToInt();gravity=Gravity.TOP or Gravity.LEFT}
        video.layoutParams=LayoutParams(config.codedWidth,config.codedHeight)
        video.pivotX=0f;video.pivotY=0f;video.scaleX=viewport.scale;video.scaleY=viewport.scale
        video.holder.setFixedSize(config.codedWidth,config.codedHeight)
    }
    override fun onSizeChanged(w:Int,h:Int,oldw:Int,oldh:Int){super.onSizeChanged(w,h,oldw,oldh);controller.viewport(w,h);layoutVideo()}
    override fun onInterceptTouchEvent(event:MotionEvent):Boolean = event.getToolType(0) in setOf(MotionEvent.TOOL_TYPE_STYLUS,MotionEvent.TOOL_TYPE_ERASER)
    override fun onTouchEvent(event:MotionEvent):Boolean = controller.touch(event)||event.getToolType(0) in setOf(MotionEvent.TOOL_TYPE_STYLUS,MotionEvent.TOOL_TYPE_ERASER)
    override fun onHoverEvent(event:MotionEvent):Boolean = controller.touch(event)||super.onHoverEvent(event)
}
