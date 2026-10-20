package com.visualworkbench.desktop.stream

import java.awt.Graphics2D

internal fun interface StreamDestinationState { fun isCurrent():Boolean }
internal interface StreamDestinationSurface {
    val identity:Any
    fun isValid():Boolean
    fun isLost():Boolean
    fun state():StreamDestinationState
    fun isSubmitted():Boolean
}

/** The operation epoch starts before the first destination write and spans
 * implicit updater calls inside drawImage as well as explicit sync. The later
 * pixel-state tracker alone cannot detect a frame cleared before its capture. */
internal fun submitStreamDestination(graphics:Graphics2D,operation:StreamDestinationState,
    draw:(Graphics2D)->Boolean,sync:()->Unit,
    observe:(Graphics2D)->StreamDestinationSurface?=JdkStreamDestination::observe){
    check(operation.isCurrent())
    val before=checkNotNull(observe(graphics)){"Desktop stream destination evidence is unavailable"}
    check(before.isValid()&&!before.isLost())
    check(operation.isCurrent())
    check(draw(graphics))
    check(operation.isCurrent()){"Desktop stream destination changed during drawing"}
    val rendered=checkNotNull(observe(graphics))
    check(rendered.identity===before.identity&&rendered.isValid()&&!rendered.isLost())
    val state=rendered.state()
    check(state.isCurrent()&&operation.isCurrent())
    sync()
    check(operation.isCurrent())
    val after=checkNotNull(observe(graphics))
    check(after.identity===rendered.identity&&after.isValid()&&!after.isLost()&&
        state.isCurrent()&&after.isSubmitted()&&operation.isCurrent()){"Desktop stream destination did not confirm submission"}
}

/** OpenJDK 21 destination evidence; no access bypass or guessed success.
 * Root runtime/test options must include:
 * --add-exports=java.desktop/sun.java2d=ALL-UNNAMED
 * --add-exports=java.desktop/sun.java2d.d3d=ALL-UNNAMED (Windows)
 * Missing exports or an unavailable observation produce unknown presentation. */
private object JdkStreamDestination {
    private class Access {
        val graphics=Class.forName("sun.java2d.SunGraphics2D")
        val surface=Class.forName("sun.java2d.SurfaceData")
        val tracker=Class.forName("sun.java2d.StateTracker")
        val getSurface=graphics.getMethod("getSurfaceData")
        val valid=surface.getMethod("isValid")
        val lost=surface.getMethod("isSurfaceLost")
        val state=surface.getMethod("getStateTracker")
        val current=tracker.getMethod("isCurrent")
        init {check(surface.module.isExported("sun.java2d",JdkStreamDestination::class.java.module))}
        fun wrap(value:Any):StreamDestinationSurface=object:StreamDestinationSurface {
            override val identity=value
            override fun isValid()=valid.invoke(value) as Boolean
            override fun isLost()=lost.invoke(value) as Boolean
            override fun state():StreamDestinationState {
                val captured=state.invoke(value)
                return StreamDestinationState{current.invoke(captured) as Boolean}
            }
            override fun isSubmitted():Boolean {
                if(value.javaClass.name=="sun.java2d.d3d.D3DSurfaceData\$D3DWindowSurfaceData"){
                    // D3D swap marks clean without changing rendered state;
                    // restoration also marks clean but invalidates that state.
                    return !(value.javaClass.getMethod("isDirty").invoke(value) as Boolean)
                }
                return true
            }
        }
    }
    private val access by lazy {runCatching{Access()}.getOrNull()}
    fun observe(graphics:Graphics2D):StreamDestinationSurface?=runCatching {
        val api=access?:return null
        if(!api.graphics.isInstance(graphics))return null
        api.wrap(api.getSurface.invoke(graphics))
    }.getOrNull()
}
