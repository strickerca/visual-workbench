package com.visualworkbench.shared

import com.visualworkbench.bindings.core.CaptureLocalOptions as NOptions
import com.visualworkbench.bindings.core.CaptureLocalRect as NRect
import com.visualworkbench.bindings.core.CaptureLocalWindow as NWindow
import com.visualworkbench.bindings.core.CaptureLocalMonitor as NMonitor
import com.visualworkbench.bindings.core.CaptureLocalSelection as NSelection
import com.visualworkbench.bindings.core.CaptureLocalProducer as NProducer
import com.visualworkbench.bindings.core.CaptureLocalProgress as NProgress
import com.visualworkbench.bindings.core.Cancellation as NCancellation
import com.visualworkbench.bindings.core.SessionException
import kotlinx.coroutines.*

private fun CaptureLocalRect.native():NRect=NRect(x,y,width,height)
private fun CaptureLocalMonitor.native():NMonitor=NMonitor(monitor,device,physical.native(),dpi,rotation,primary,observedNs)
private fun CaptureLocalSelection.native():NSelection=when(this){
    is CaptureLocalSelection.Window->NSelection.Window(NWindow(target.window,target.processId,target.processCreated,
        target.client.native(),target.frame.native(),target.dpi,target.observedNs))
    is CaptureLocalSelection.Monitor->NSelection.Monitor(target.native())
    is CaptureLocalSelection.Region->NSelection.Region(target.native(),region.native())
}
internal suspend fun <T> localCaptureCall(block:suspend()->T):T=withContext(Dispatchers.Default){
    try{block()}catch(error:SessionException){throw sessionFailure(error)}
    catch(_:IllegalStateException){throw SessionFailure(SessionFailureKind.Closed)}
}
public actual suspend fun WorkbenchCapturePublisher.openLocalCapture(options:CaptureLocalOptions):WorkbenchLocalCapture {
    val publisher=this as? NativeCapturePublisher?:throw SessionFailure(SessionFailureKind.Invalid)
    val cancel=NCancellation()
    try {
        return publisher.openLocal(cancel::cancel){ handle, retired->
            NativeLocalCapture(localCaptureCall{handle.openLocalCapture(NOptions(options.explicitOwnerAction,
                options.selection.native(),options.helperPath,options.trustedHelperSha256,options.ringBytes),cancel)},retired)
        }
    } finally { cancel.destroy() }
}
private class NativeLocalCapture(private val handle:NProducer,private val retired:()->Unit):WorkbenchLocalCapture,OwnedSession {
    private val owner=CaptureNativeOwner()
    override suspend fun pumpOnce():CaptureLocalProgress=owner.call {
        joinedCaptureCall(cancel=handle::cancel){localCaptureCall{when(val next=handle.pumpOnce()){
            NProgress.Idle->CaptureLocalProgress(CaptureLocalProgressKind.Idle)
            is NProgress.Tiles->CaptureLocalProgress(CaptureLocalProgressKind.Tiles,next.frameId,next.admitted)
            is NProgress.OriginalProvided->CaptureLocalProgress(CaptureLocalProgressKind.OriginalProvided,next.frameId)
            NProgress.OriginalUnavailable->CaptureLocalProgress(CaptureLocalProgressKind.OriginalUnavailable)
        }}}
    }
    override fun cancel():Unit=owner.now{handle.cancel()}
    override suspend fun close():Unit {
        owner.close({localCaptureCall{handle.closeProducer()}},handle::destroy)
        retired()
    }
}
