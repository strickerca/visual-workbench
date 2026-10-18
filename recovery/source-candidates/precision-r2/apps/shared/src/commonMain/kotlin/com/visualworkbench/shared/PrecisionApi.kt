package com.visualworkbench.shared

public data class PrecisionMapping(public val base:Transform,public val lens:Transform,
    public val open:Boolean,public val latched:Boolean,public val lensInput:Boolean)
/** Synchronous bounded geometry; owns no project, provider or native worker. */
public interface WorkbenchPrecision {
    public fun configure(focus:Point,center:Point,magnification:Int)
    public fun closeLens()
    public fun down(inside:Boolean)
    public fun up()
    public fun mapToBase(points:List<Point>,inside:Boolean=false):List<Point>
    public fun mapping():PrecisionMapping
    public fun viewToBase(points:List<Point>):List<Point>
    public fun close()
}
public expect fun precisionGeometry(camera:Camera):WorkbenchPrecision
public expect fun precisionViewTransform(camera:Camera,focus:Point,center:Point,magnification:Int):Transform
