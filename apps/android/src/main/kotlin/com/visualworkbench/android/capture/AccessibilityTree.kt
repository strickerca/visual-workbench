@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
package com.visualworkbench.android.capture

import android.accessibilityservice.AccessibilityService
import android.graphics.Rect
import android.os.SystemClock
import android.view.accessibility.AccessibilityNodeInfo
import com.visualworkbench.shared.CapturedSemanticElement
import com.visualworkbench.shared.Rect as CoreRect
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resumeWithException

internal data class CapturedTree(val elements:List<CapturedSemanticElement>,val observedMs:Long,val elapsedMs:Long)
/** Provider Binder calls cannot be interrupted reliably. One worker remains
 * occupied until the actual call returns; timeout never creates a replacement
 * thread or releases an owned node early. A late result is only managed data. */
internal object AccessibilityTree {
    private val busy=AtomicBoolean()
    private val worker=ThreadPoolExecutor(1,1,0,TimeUnit.MILLISECONDS,ArrayBlockingQueue(1)){task->Thread(task,"vw-capture-tree").apply{isDaemon=true}}
    suspend fun collect(service:AccessibilityService,target:Target):CapturedTree = submit { cancelled->read(service,target,cancelled) }
    @Suppress("DEPRECATION")
    suspend fun inspect(service:AccessibilityService):Target=submit{cancelled->
        if(cancelled.get())throw CaptureRefusal(CaptureRefusal.Reason.Timeout)
        val root=service.rootInActiveWindow?:throw CaptureRefusal(CaptureRefusal.Reason.Unavailable)
        try{CaptureAdmission.target(CaptureService.targetOf(service,root),service.packageName)}finally{root.recycle()}
    }
    private suspend fun <T> submit(block:(AtomicBoolean)->T):T = suspendCancellableCoroutine { continuation->
        if(!busy.compareAndSet(false,true)){continuation.resumeWithException(CaptureRefusal(CaptureRefusal.Reason.Busy));return@suspendCancellableCoroutine}
        val cancelled=AtomicBoolean()
        continuation.invokeOnCancellation{cancelled.set(true)}
        try{worker.execute{
            try{val tree=block(cancelled);if(continuation.isActive)continuation.resume(tree,onCancellation={_,_,_->})}
            catch(error:Exception){if(continuation.isActive)continuation.resumeWithException(error)}
            finally{busy.set(false)}
        }}catch(error:Exception){busy.set(false);continuation.resumeWithException(error)}
    }
    @Suppress("DEPRECATION")
    private fun read(service:AccessibilityService,target:Target,cancelled:AtomicBoolean):CapturedTree {
        val started=SystemClock.uptimeMillis()
        fun check(){if(cancelled.get()||SystemClock.uptimeMillis()-started>300)throw CaptureRefusal(CaptureRefusal.Reason.Timeout)}
        val root=service.rootInActiveWindow?:throw CaptureRefusal(CaptureRefusal.Reason.Unavailable)
        data class Cursor(val node:AccessibilityNodeInfo,val parent:String?,val depth:Int,var next:Int=0,var id:String?=null)
        val stack=ArrayList<Cursor>();stack.add(Cursor(root,null,0));val result=ArrayList<CapturedSemanticElement>();var bytes=0
        try{
            target.same(CaptureService.targetOf(service,root))
            while(stack.isNotEmpty()){
                check();val cursor=stack.last();val node=cursor.node
                if(cursor.id==null){
                    if(result.size>=CaptureAdmission.ELEMENTS||cursor.depth>=128||node.childCount>CaptureAdmission.ELEMENTS)throw CaptureRefusal(CaptureRefusal.Reason.Limit)
                    if(node.windowId!=target.windowId)throw CaptureRefusal(CaptureRefusal.Reason.Stale)
                    val rect=Rect();node.getBoundsInScreen(rect)
                    val id="node-${result.size}";cursor.id=id
                    val description=CaptureAdmission.text(node.contentDescription,4096)
                    val role=CaptureAdmission.text(node.className,256)
                    val resource=CaptureAdmission.text(node.viewIdResourceName,1024).ifEmpty{null}
                    val text=if(node.isPassword)"" else CaptureAdmission.excerpt(node.text)
                    val value=CapturedSemanticElement(id,cursor.parent,description.ifEmpty{text},role,null,resource,null,
                        CoreRect(rect.left.toDouble(),rect.top.toDouble(),rect.width().toDouble(),rect.height().toDouble()),text,node.isEnabled,node.isFocused)
                    bytes=CaptureAdmission.element(value,bytes);result.add(value)
                }
                if(cursor.next<node.childCount){val child=node.getChild(cursor.next++);if(child!=null)stack.add(Cursor(child,cursor.id,cursor.depth+1));check()}
                else{stack.removeAt(stack.lastIndex);node.recycle()}
            }
            check();val current=service.rootInActiveWindow?:throw CaptureRefusal(CaptureRefusal.Reason.Stale)
            try{target.same(CaptureService.targetOf(service,current))}finally{current.recycle()}
            check();return CapturedTree(result,started,SystemClock.uptimeMillis()-started)
        }finally{stack.forEach{it.node.recycle()}}
    }
}
