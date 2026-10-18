package com.visualworkbench.shared

import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.hypot

/** Render-only integer D-pixel edge grid. Hard cap prevents zoom-out work growth;
 * this function never maps input. It supports rotated/sheared finite matrices. */
public fun precisionPixelGrid(pane:MagnifierPane):List<Pair<Point,Point>> {
    val t=pane.matrix;val det=t.a*t.d-t.b*t.c
    if(!det.isFinite()||det==0.0||minOf(hypot(t.a,t.b),hypot(t.c,t.d))<8.0)return emptyList()
    fun inverse(p:Point):Point {val x=p.x-t.e;val y=p.y-t.f;return Point((t.d*x-t.c*y)/det,(-t.b*x+t.a*y)/det)}
    fun forward(p:Point):Point=Point(t.a*p.x+t.c*p.y+t.e,t.b*p.x+t.d*p.y+t.f)
    val r=pane.rectangle
    val points=listOf(Point(r.x,r.y),Point(r.x+r.width,r.y),Point(r.x+r.width,r.y+r.height),Point(r.x,r.y+r.height)).map(::inverse)
    if(points.any{!it.x.isFinite()||!it.y.isFinite()})return emptyList()
    val x0=floor(points.minOf{it.x});val x1=ceil(points.maxOf{it.x});val y0=floor(points.minOf{it.y});val y1=ceil(points.maxOf{it.y})
    if(x1-x0+y1-y0>256||listOf(x0,x1,y0,y1).any{it < -1e9 || it > 1e9})return emptyList()
    val lines=mutableListOf<Pair<Point,Point>>()
    for(x in x0.toLong()..x1.toLong())lines+=forward(Point(x.toDouble(),y0)) to forward(Point(x.toDouble(),y1))
    for(y in y0.toLong()..y1.toLong())lines+=forward(Point(x0,y.toDouble())) to forward(Point(x1,y.toDouble()))
    return lines
}
