package com.visualworkbench.desktop.stream

import java.awt.Color
import java.awt.image.BufferedImage
import kotlinx.coroutines.runBlocking

/** Headless actual-JDK destination state check. Use
 * --add-exports=java.desktop/sun.java2d=ALL-UNNAMED.
 * This projects the reviewed D3D restoration state change; it is not Windows
 * failure/device evidence. --expect-unavailable verifies missing-access refusal. */
internal object DesktopStreamDestinationJdkWitness {
    @JvmStatic fun main(args:Array<String>)=runBlocking {
        val image=BufferedImage(2,2,BufferedImage.TYPE_INT_RGB)
        val graphics=image.createGraphics()
        fun draw():Boolean {graphics.color=Color.RED;graphics.fillRect(0,0,2,2);return true}
        try {
            val normal=StreamDrawPermit()
            val attempted=runCatching{normal.paint{submitStreamDestination(graphics,{draw()},{})}}
            if(args.contains("--expect-unavailable")){
                check(attempted.isFailure&&normal.awaitDecision()==null)
                println("Missing destination exports refuse positive receipt as unknown")
                return@runBlocking
            }
            check(attempted.getOrThrow()&&normal.awaitDecision()==true)
            val getSurface=Class.forName("sun.java2d.SunGraphics2D").getMethod("getSurfaceData")
            val surface=getSurface.invoke(graphics)
            val lost=Class.forName("sun.java2d.SurfaceData").getMethod("setSurfaceLost",Boolean::class.javaPrimitiveType)
            val restored=StreamDrawPermit()
            check(runCatching{restored.paint{submitStreamDestination(graphics,{draw()},{
                lost.invoke(surface,true);lost.invoke(surface,false)
            })}}.isFailure)
            check(restored.awaitDecision()==null)
            val repaint=StreamDrawPermit()
            check(runCatching{repaint.paint{submitStreamDestination(graphics,{draw()},{
                graphics.color=Color.BLACK;graphics.fillRect(0,0,2,2)
            })}}.isFailure)
            check(repaint.awaitDecision()==null)
            println("JDK destination witness passed: normal=true; lost/restored=null; repaint-only=null")
        }finally{graphics.dispose();image.flush()}
    }
}
