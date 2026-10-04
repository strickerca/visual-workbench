@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
package com.visualworkbench.android.capture

import android.accessibilityservice.AccessibilityService
import android.graphics.Bitmap
import android.graphics.ColorSpace
import android.graphics.Rect
import android.hardware.HardwareBuffer
import android.os.Build
import android.os.SystemClock
import android.view.Display
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.Toast
import androidx.annotation.RequiresApi
import java.io.FileOutputStream
import java.io.FilterOutputStream
import java.io.OutputStream
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.*
import kotlin.coroutines.resumeWithException

/** Enabling the service never captures anything. Accessibility events are
 * ignored; only Tile/notification/shortcut confirmation invokes request(). */
class CaptureService:AccessibilityService() {
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
    override fun onAccessibilityEvent(event:AccessibilityEvent?){ }
    override fun onInterrupt(){scope.coroutineContext.cancelChildren()}
    override fun onServiceConnected(){instance.set(this);CaptureEntrypoints.notification(this)}
    override fun onDestroy(){instance.compareAndSet(this,null);scope.cancel();super.onDestroy()}
    internal fun request(){
        if(!scope.isActive)return
        if(Build.VERSION.SDK_INT<30){notice("Screen capture requires Android 11 or later.");return}
        if(CaptureRuntime.installed.get()==null){notice("Open Visual Workbench before capturing.");return}
        if(!captureActive.compareAndSet(false,true)){notice("A capture is already running.");return}
        scope.launch {
            var lease:CaptureLease?=null
            try{
                if(Build.VERSION.SDK_INT>=31)performGlobalAction(GLOBAL_ACTION_DISMISS_NOTIFICATION_SHADE)
                // Shade dismissal is explicit-owner UI cleanup, never an app
                // click or back injection. API30 waits for owner dismissal.
                var target:Target?=null
                repeat(20){if(target==null){target=runCatching{currentTarget()}.getOrNull();if(target==null)delay(50)}}
                val bound=target?:throw CaptureRefusal(CaptureRefusal.Reason.Unavailable)
                lease=CaptureLease.create(this@CaptureService)
                val frame=screenshot(bound,lease)
                val runtime=CaptureRuntime.installed.get()?:throw CaptureRefusal(CaptureRefusal.Reason.Unavailable)
                runtime.importAndCollect(lease,frame){withTimeout(350){AccessibilityTree.collect(this@CaptureService,bound)}}
                lease=null
            }catch(_:CancellationException){notice("Capture cancelled. Any completed original remains in the private capture inbox.")}
            catch(error:CaptureRefusal){notice("Capture unavailable: ${error.reason.name}. Completed originals remain in the private capture inbox.")}
            catch(_:Exception){notice("Capture could not be completed. Completed originals remain in the private capture inbox.")}
            finally{
                // Never discard the owner's completed screenshot on an import
                // failure. Empty leases contain no captured information.
                lease?.let{if(!it.png.exists()||it.png.length()==0L)runCatching{it.release()}}
                captureActive.set(false)
            }
        }
    }
    private fun notice(message:String){Toast.makeText(this,message,Toast.LENGTH_LONG).show()}
    private suspend fun currentTarget():Target=withTimeout(350){AccessibilityTree.inspect(this@CaptureService)}
    private suspend fun screenshot(bound:Target,lease:CaptureLease):CaptureFrame {
        if(Build.VERSION.SDK_INT<30)throw CaptureRefusal(CaptureRefusal.Reason.Unavailable)
        return screenshotApi30(bound,lease)
    }
    @RequiresApi(30)
    private suspend fun screenshotApi30(bound:Target,lease:CaptureLease):CaptureFrame {
        val result=withTimeout(3000){suspendCancellableCoroutine<ScreenshotResult>{continuation->
            if(!continuation.isActive)return@suspendCancellableCoroutine
            if(!screenshotInFlight.compareAndSet(false,true)){continuation.resumeWithException(CaptureRefusal(CaptureRefusal.Reason.Busy));return@suspendCancellableCoroutine}
            try{takeScreenshot(Display.DEFAULT_DISPLAY,mainExecutor,object:TakeScreenshotCallback {
                override fun onSuccess(value:ScreenshotResult){
                    try{if(!continuation.isActive){value.hardwareBuffer.close();return}
                        continuation.resume(value,onCancellation={_,lost,_->lost.hardwareBuffer.close()})
                    }finally{screenshotInFlight.set(false)}
                }
                override fun onFailure(code:Int){try{if(continuation.isActive)continuation.resumeWithException(CaptureRefusal(if(Build.VERSION.SDK_INT>=34&&code==ERROR_TAKE_SCREENSHOT_SECURE_WINDOW)CaptureRefusal.Reason.Secure else CaptureRefusal.Reason.Unavailable))}finally{screenshotInFlight.set(false)}}
            })}catch(error:Exception){screenshotInFlight.set(false);if(continuation.isActive)continuation.resumeWithException(error)}
        }}
        val hardware=result.hardwareBuffer
        try{
            bound.same(currentTarget());CaptureAdmission.pixels(hardware.width,hardware.height)
            if(bound.bounds.left<0||bound.bounds.top<0||bound.bounds.right>hardware.width||bound.bounds.bottom>hardware.height)throw CaptureRefusal(CaptureRefusal.Reason.Stale)
            if(hardware.format!=HardwareBuffer.RGBA_8888||result.colorSpace!=ColorSpace.get(ColorSpace.Named.SRGB))throw CaptureRefusal(CaptureRefusal.Reason.ColorDepth)
            val wrapped=Bitmap.wrapHardwareBuffer(hardware,result.colorSpace)?:throw CaptureRefusal(CaptureRefusal.Reason.Unavailable)
            try{
                // Copy/scan/encode off the UI thread. Only managed frame data
                // crosses the dispatcher return; every Bitmap stays owned and
                // is recycled in that worker even if cancellation wins delivery.
                val frame=withContext(NonCancellable+Dispatchers.Default){
                    val bitmap=wrapped.copy(Bitmap.Config.ARGB_8888,false)?:throw CaptureRefusal(CaptureRefusal.Reason.Memory)
                    try{
                        val row=IntArray(bitmap.width)
                        for(y in 0 until bitmap.height){bitmap.getPixels(row,0,bitmap.width,0,y,bitmap.width,1);if(row.any{it ushr 24!=255})throw CaptureRefusal(CaptureRefusal.Reason.ColorDepth)}
                        check(lease.png.createNewFile())
                        FileOutputStream(lease.png).use{file->val output=LimitedOutput(file,CaptureAdmission.ENCODED);check(bitmap.compress(Bitmap.CompressFormat.PNG,100,output));output.flush();file.fd.sync()}
                        val age=SystemClock.uptimeMillis()-result.timestamp
                        if(age<0||age>10_000)throw CaptureRefusal(CaptureRefusal.Reason.Stale)
                        CaptureFrame(bitmap.width,bitmap.height,result.timestamp,System.currentTimeMillis()-age)
                    }finally{bitmap.recycle()}
                }
                currentCoroutineContext().ensureActive();bound.same(currentTarget());return frame
            }finally{wrapped.recycle()}
        }finally{hardware.close()}
    }
    companion object {
        private val captureActive=AtomicBoolean()
        // A timed-out framework request owns this slot until its real callback.
        // Re-enabling/recreating the service cannot queue replacement requests.
        private val screenshotInFlight=AtomicBoolean()
        internal val instance=AtomicReference<CaptureService?>(null)
        @Suppress("DEPRECATION")
        internal fun targetOf(service:AccessibilityService,node:AccessibilityNodeInfo):Target {
            if(Build.VERSION.SDK_INT<30)throw CaptureRefusal(CaptureRefusal.Reason.Unavailable)
            val window=node.window?:throw CaptureRefusal(CaptureRefusal.Reason.Unavailable)
            try{if(window.displayId!=Display.DEFAULT_DISPLAY||window.type!=android.view.accessibility.AccessibilityWindowInfo.TYPE_APPLICATION)throw CaptureRefusal(CaptureRefusal.Reason.Unavailable)}finally{window.recycle()}
            val rect=Rect();node.getBoundsInScreen(rect)
            val manager=service.getSystemService(android.view.WindowManager::class.java)
            return Target(node.windowId,CaptureAdmission.text(node.packageName,1024),rect,manager.defaultDisplay.rotation)
        }
    }
}
internal class LimitedOutput(output:OutputStream,private val limit:Long):FilterOutputStream(output){
    private var count=0L
    override fun write(value:Int){admit(1);out.write(value)}
    override fun write(value:ByteArray,offset:Int,length:Int){admit(length);out.write(value,offset,length)}
    private fun admit(length:Int){count=Math.addExact(count,length.toLong());if(count>limit)throw CaptureRefusal(CaptureRefusal.Reason.Limit)}
}
