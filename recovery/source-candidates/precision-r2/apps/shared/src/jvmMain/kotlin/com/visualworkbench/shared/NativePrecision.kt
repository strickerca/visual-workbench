package com.visualworkbench.shared

import com.visualworkbench.bindings.core.CoreException
import com.visualworkbench.bindings.core.PrecisionLens
import com.visualworkbench.bindings.core.createPrecisionLens
import com.visualworkbench.bindings.core.Camera as NCamera
import com.visualworkbench.bindings.core.Point as NPoint
import com.visualworkbench.bindings.core.Transform as NTransform

public actual fun precisionGeometry(camera:Camera):WorkbenchPrecision {
    prepareCoreLibrary()
    return translate { NativePrecision(createPrecisionLens(NCamera(NPoint(camera.center.x,camera.center.y),camera.scale,camera.rotation,camera.viewportWidth,camera.viewportHeight))) }
}
public actual fun precisionViewTransform(camera:Camera,focus:Point,center:Point,magnification:Int):Transform = translate {
    require(magnification in listOf(2,4,8));prepareCoreLibrary()
    com.visualworkbench.bindings.core.precisionViewTransform(NCamera(camera.center.native(),camera.scale,camera.rotation,camera.viewportWidth,camera.viewportHeight),focus.native(),center.native(),magnification.toUByte()).common()
}
private inline fun <T> translate(action:()->T):T = try { action() } catch(error:CoreException) {throw mapFailure(error)}
private fun Point.native():NPoint=NPoint(x,y)
private fun NTransform.common():Transform=Transform(a,b,c,d,e,f)
private class NativePrecision(private val handle:PrecisionLens):WorkbenchPrecision {
    private var closed=false
    private fun <T> call(action:()->T):T {if(closed)throw CoreFailure(CoreFailureKind.Closed);return translate(action)}
    override fun configure(focus:Point,center:Point,magnification:Int):Unit=call { require(magnification in listOf(2,4,8));handle.configure(focus.native(),center.native(),magnification.toUByte()) }
    override fun closeLens():Unit=call {handle.closeLens()}
    override fun down(inside:Boolean):Unit=call {handle.down(inside)}
    override fun up():Unit=call {handle.up()}
    override fun mapToBase(points:List<Point>,inside:Boolean):List<Point> = call {
        require(points.size<=512);handle.mapToBase(points.map{it.native()},inside).map{Point(it.x,it.y)}
    }
    override fun mapping():PrecisionMapping=call {handle.mapping().let{PrecisionMapping(it.base.common(),it.lens.common(),it.open,it.latched,it.lensInput)}}
    override fun viewToBase(points:List<Point>):List<Point> = call {require(points.size<=512);handle.viewToBase(points.map{it.native()}).map{Point(it.x,it.y)}}
    override fun close() {if(!closed){translate{handle.dispose()};closed=true;handle.destroy()}}
}
