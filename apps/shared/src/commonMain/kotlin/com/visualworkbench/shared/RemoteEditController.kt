package com.visualworkbench.shared

import kotlin.math.floor
import kotlin.math.min

/** Letterbox uses visible pixels only. The coded padding row/column never
 * belongs to the displayed F-space or to an input destination. */
public data class RemoteViewport(public val left:Float,public val top:Float,public val width:Float,public val height:Float,public val scale:Float) {
    public fun host(x:Float,y:Float,rect:RemotePhysicalRect):Pair<Int,Int>? {
        if(!x.isFinite()||!y.isFinite()||scale<=0f||x<left||y<top||x>=left+width||y>=top+height)return null
        val dx=floor((x-left)/scale).toLong();val dy=floor((y-top)/scale).toLong()
        if(dx !in 0 until rect.width.toLong()||dy !in 0 until rect.height.toLong())return null
        val hx=rect.x.toLong()+dx;val hy=rect.y.toLong()+dy
        if(hx !in Int.MIN_VALUE.toLong()..Int.MAX_VALUE.toLong()||hy !in Int.MIN_VALUE.toLong()..Int.MAX_VALUE.toLong())return null
        return hx.toInt() to hy.toInt()
    }
    public companion object {
        public fun fit(viewWidth:Int,viewHeight:Int,visibleWidth:Int,visibleHeight:Int):RemoteViewport? {
            if(viewWidth<=0||viewHeight<=0||visibleWidth !in 1..4096||visibleHeight !in 1..4096)return null
            val scale=min(viewWidth.toFloat()/visibleWidth,viewHeight.toFloat()/visibleHeight)
            val width=floor(visibleWidth*scale);val height=floor(visibleHeight*scale)
            return RemoteViewport(floor((viewWidth-width)/2),floor((viewHeight-height)/2),width,height,scale)
        }
    }
}
/** Actual admission only. Each new segment gets its own creation deadline;
 * an overlap anchor carries no event/sequence and never refreshes old expiry. */
public class RemoteGhostSegments(private val ink:RemoteGhostInk) {
    private var current:GhostSegment?=null
    public fun admitted(admission:RemoteInputAdmission,point:GhostPoint,contact:Boolean):Boolean {
        if(point.predicted||point.anchor||point.localNanos!=admission.sampledLocalNanos||admission.sequence==0uL)return false
        ink.activate(admission.binding,point.localNanos)
        val old=current?.takeIf{it.binding==admission.binding}
        if(old!=null&&admission.sequence<=old.lastInputSequence)return false
        if(!contact&&old==null)return true
        val newSegment=old==null||point.localNanos<old.createdLocalNanos||point.localNanos-old.createdLocalNanos>=100_000_000L||old.points.count{!it.predicted&&!it.anchor}>=63
        val next=if(newSegment){val anchor=old?.points?.lastOrNull{!it.predicted&&!it.anchor}?.copy(anchor=true);GhostSegment(admission.binding,admission.sequence,admission.sequence,point.localNanos,listOfNotNull(anchor,point))}
            else {if(admission.sequence<=old.lastInputSequence)return false;old.copy(lastInputSequence=admission.sequence,points=old.points.filterNot{it.predicted}+point)}
        val accepted=if(newSegment)ink.add(next)else ink.update(next)
        current=if(contact&&accepted)next else null
        return accepted
    }
    public fun retire(binding:RemoteTargetBinding?=null){ink.retire(binding);if(binding==null||current?.binding==binding)current=null}
    public fun prediction(point:GhostPoint):Boolean {
        val old=current?:return false;val actual=old.points.lastOrNull{!it.predicted&&!it.anchor}?:return false
        if(!point.predicted||point.anchor||point.localNanos<actual.localNanos||point.localNanos-actual.localNanos>20_000_000L||
            !point.x.isFinite()||!point.y.isFinite()||kotlin.math.abs(point.x-actual.x)>64f||kotlin.math.abs(point.y-actual.y)>64f)return false
        val next=old.copy(points=old.points.filterNot{it.predicted}+point)
        if(next.points.size>64||!ink.update(next))return false;current=next;return true
    }
}
