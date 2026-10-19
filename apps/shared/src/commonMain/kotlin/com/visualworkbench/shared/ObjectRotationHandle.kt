package com.visualworkbench.shared

import kotlin.math.hypot

/** D-space display/hit geometry only. The rotation solver remains canonical
 * native geometry; the existing precision route supplies the inverse input. */
public data class ObjectRotationHandle(public val pivot:Point,public val point:Point)
public fun objectRotationHandle(document:DocumentSnapshot,selected:Set<String>,camera:Camera,
    offsetPixels:Double=28.0):ObjectRotationHandle? {
    if(!camera.scale.isFinite()||camera.scale<=0.0||!offsetPixels.isFinite()||offsetPixels<=0.0)return null
    val items=document.render.items.filter{it.objectId in selected}
    if(items.size!=selected.size||!objectRotationAllowed(items))return null
    val left=items.minOf{it.bounds.x};val top=items.minOf{it.bounds.y}
    val right=items.maxOf{it.bounds.x+it.bounds.width};val bottom=items.maxOf{it.bounds.y+it.bounds.height}
    if(!listOf(left,top,right,bottom).all{it.isFinite()}||right<left||bottom<top)return null
    val pivot=Point((left+right)/2.0,(top+bottom)/2.0)
    val point=Point(pivot.x,top-offsetPixels/camera.scale)
    return if(listOf(pivot.x,pivot.y,point.x,point.y).all{it.isFinite()})ObjectRotationHandle(pivot,point)else null
}
public fun ObjectRotationHandle.hit(physical:Transform,raw:Point,radiusPixels:Double):Boolean {
    if(!radiusPixels.isFinite()||radiusPixels<=0.0||!listOf(physical.a,physical.b,physical.c,physical.d,physical.e,physical.f,raw.x,raw.y).all{it.isFinite()})return false
    val x=physical.a*point.x+physical.c*point.y+physical.e
    val y=physical.b*point.x+physical.d*point.y+physical.f
    return x.isFinite()&&y.isFinite()&&hypot(raw.x-x,raw.y-y)<=radiusPixels
}
