package com.visualworkbench.shared

import com.visualworkbench.bindings.core.LiveSession
import com.visualworkbench.bindings.core.RemoteDrawingPreparation as ForeignDrawingPreparation
import com.visualworkbench.bindings.core.RemoteDrawingCanvas as ForeignCanvas
import com.visualworkbench.bindings.core.SessionException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Normal factory access only. All source, input reservation, UIA, WGC and
 * evidence production remain in actual native owners and the pinned helper. */
internal class NativeDrawingRuntimeAccess(private val handle:LiveSession):NativeDrawingPreparationAccess {
    override val available:Boolean get()=try{handle.remoteDrawingCheckAvailable()}catch(_:SessionException){false}
    override suspend fun prepare(expected:RemoteTargetState):NativeDrawingPreparation = withContext(NonCancellable) {
        try {
            val owned=handle.remotePrepareDrawingCheck(RemoteNativeJson.binding(expected.binding))
            // Return allocation before any metadata/projection can fail. The
            // session owner adopts this wrapper before reading target/readiness.
            Prepared(expected,owned)
        }catch(error:SessionException){throw sessionFailure(error)}
    }
    private class Prepared(private val expected:RemoteTargetState,private val owned:ForeignDrawingPreparation):NativeDrawingPreparation {
        private var destroyed=false
        private var retirement:RemoteDrawingCanvasState?=null
        private var destroyFailure:Throwable?=null
        private var runStarted=false
        @Volatile private var recoveryNeeded=false
        override val canvasRecoveryNeeded:Boolean get()=recoveryNeeded
        override val target:RemoteTargetState get() {
            check(!destroyed)
            require(RemoteNativeJson.binding(RemoteNativeJson.read(owned.bindingJson()))==expected.binding)
            val image=RemoteNativeJson.read(owned.identityJson())
            require(image.keys==setOf("executableName","executableBlake3","executableBytes","fileVersion","packageFullName","packageVersion"))
            val actual=RemoteEditorIdentity(RemoteEditorKind.Paint,RemoteNativeJson.string(image["executableName"]),
                RemoteNativeJson.string(image["executableBlake3"]),RemoteNativeJson.ulong(image["executableBytes"]),
                RemoteNativeJson.optional(image["fileVersion"]),RemoteNativeJson.optional(image["packageFullName"]),
                RemoteNativeJson.optional(image["packageVersion"]),owned.profileDigest())
            require(actual==expected.identity&&actual.valid())
            return expected
        }
        // Read once after adoption; no foreign getter after retirement/destroy.
        private var ready:Boolean?=null
        override val readyForConfirmation:Boolean get() {
            ready?.let{return it};check(!destroyed)
            return owned.preparedReady().also{ready=it}
        }
        override fun cancel(){if(!destroyed)owned.cancel()}
        override suspend fun runConfirmed():NativeOwnedCompatibilityReceipt {
            check(!destroyed&&readyForConfirmation)
            runStarted=true
            try {
                val bytes=owned.runConfirmed()
                val value=NativeRemoteCompatibility.decode(expected,bytes)
                return NativeOwnedCompatibilityReceipt(expected,value.toolId,value.settingsDigest,bytes,value.records)
            }catch(error:SessionException){throw sessionFailure(error)}
        }
        override suspend fun retire():RemoteDrawingCanvasState = withContext(NonCancellable+Dispatchers.Default) {
            destroyFailure?.let{throw it}
            retirement?.let{return@withContext it}
            check(!destroyed)
            try {
                val canvas=when(owned.retire()){
                    ForeignCanvas.UNCHANGED->RemoteDrawingCanvasState.Unchanged
                    ForeignCanvas.RESTORED->RemoteDrawingCanvasState.Restored
                    ForeignCanvas.RECOVERY_NEEDED->RemoteDrawingCanvasState.RecoveryNeeded
                    ForeignCanvas.UNKNOWN->RemoteDrawingCanvasState.Unknown
                }
                recoveryNeeded=canvas in setOf(RemoteDrawingCanvasState.RecoveryNeeded,RemoteDrawingCanvasState.Unknown)
                // A failed foreign destroy has unknown consumption. Keep that
                // error/owner and never call through the same handle again.
                destroyed=true
                try{owned.destroy()}catch(error:Throwable){destroyFailure=error;throw error}
                retirement=canvas
                canvas
            }catch(error:SessionException){
                if(!destroyed)recoveryNeeded=try {owned.canvasState() in setOf(ForeignCanvas.UNKNOWN,ForeignCanvas.RECOVERY_NEEDED)}
                    catch(_:Exception){runStarted}
                throw sessionFailure(error)
            }
        }
    }
}
