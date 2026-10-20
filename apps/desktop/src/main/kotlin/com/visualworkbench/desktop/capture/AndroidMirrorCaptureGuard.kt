package com.visualworkbench.desktop.capture

import com.visualworkbench.bindings.host.CaptureSelection
import com.visualworkbench.shared.*

internal fun CaptureSelection.tunnelSource():TunnelSource=when(this){
    is CaptureSelection.Window->TunnelSource.Window(target.window)
    is CaptureSelection.Monitor->TunnelSource.Monitor(target.monitor)
    is CaptureSelection.Region->TunnelSource.Region(target.monitor,x,y,width,height)
}

/** The same native registry epoch and live mirror census surrounds actual OS
 * capture. A mirror opened and closed during capture also invalidates output. */
internal suspend fun <T> guardedMirrorCapture(source:TunnelSource,release:suspend(T)->Unit,body:suspend()->T):T=
    run {
        // Each caller already owns this work on its independent Default
        // producer. Avoid a second cancellable dispatcher handoff of T.
        val guard=beginAndroidTunnelCaptureGuard(source)
        try {
            guard.check()
            val value=body()
            try {guard.check();guard.close();value}
            catch(error:Throwable){try{release(value)}catch(cleanup:Throwable){error.addSuppressed(cleanup)};throw error}
        } finally {guard.close()}
    }
