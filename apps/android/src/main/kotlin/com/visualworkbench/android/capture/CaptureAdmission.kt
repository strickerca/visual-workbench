package com.visualworkbench.android.capture

import android.graphics.Rect
import com.visualworkbench.shared.CapturedSemanticElement

internal class CaptureRefusal(val reason:Reason):Exception(reason.name) {
    enum class Reason { OwnerAction,Unavailable,OwnWindow,Stale,Secure,ColorDepth,Memory,Limit,Timeout,Busy,Storage }
}
internal data class Target(val windowId:Int,val packageName:String,val bounds:Rect,val rotation:Int) {
    fun same(other:Target) { if(this!=other)throw CaptureRefusal(CaptureRefusal.Reason.Stale) }
}
internal object CaptureAdmission {
    const val MEMORY=256L*1024*1024
    const val ENCODED=64L*1024*1024
    const val ELEMENTS=4096
    const val TEXT_BYTES=2*1024*1024
    fun pixels(width:Int,height:Int):Long {
        val pixels=width.toLong()*height
        if(width !in 1..32768||height !in 1..32768||pixels>50_000_000||pixels*16+16*1024*1024>MEMORY)
            throw CaptureRefusal(CaptureRefusal.Reason.Memory)
        return pixels*4
    }
    fun target(value:Target,ownPackage:String):Target {
        if(value.windowId<0||value.packageName.isBlank()||value.bounds.width()<=0||value.bounds.height()<=0)
            throw CaptureRefusal(CaptureRefusal.Reason.Unavailable)
        if(value.packageName==ownPackage||value.packageName=="com.android.systemui")throw CaptureRefusal(CaptureRefusal.Reason.OwnWindow)
        return value
    }
    fun delta(frameMs:Long,observedMs:Long):Int {
        val value=Math.subtractExact(observedMs,frameMs)
        if(value !in Int.MIN_VALUE.toLong()..Int.MAX_VALUE.toLong())throw CaptureRefusal(CaptureRefusal.Reason.Limit)
        return value.toInt()
    }
    fun text(value:CharSequence?,bytes:Int):String {
        if(value==null)return ""
        if(value.length>bytes)throw CaptureRefusal(CaptureRefusal.Reason.Limit)
        return value.toString().also{if(it.toByteArray(Charsets.UTF_8).size>bytes||it.indexOf('\u0000')>=0)throw CaptureRefusal(CaptureRefusal.Reason.Limit)}
    }
    fun excerpt(value:CharSequence?):String {
        if(value==null)return ""
        // No complete String copy of an arbitrary provider's document text.
        var end=0;var points=0
        while(end<value.length&&points<200){val first=value[end++];if(Character.isHighSurrogate(first)&&end<value.length&&Character.isLowSurrogate(value[end]))end++;points++}
        return value.subSequence(0,end).toString()
    }
    fun element(value:CapturedSemanticElement,retained:Int):Int {
        val fields=listOf(value.localId,value.parentLocalId,value.name,value.role,value.automationId,value.resourceId,value.htmlId,value.text)
        val count=fields.filterNotNull().sumOf{it.toByteArray(Charsets.UTF_8).size}
        val total=Math.addExact(retained,count)
        if(total>TEXT_BYTES||value.bounds.width<0||value.bounds.height<0)throw CaptureRefusal(CaptureRefusal.Reason.Limit)
        return total
    }
}
