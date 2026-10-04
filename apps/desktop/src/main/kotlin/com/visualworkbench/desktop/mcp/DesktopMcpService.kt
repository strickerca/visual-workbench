package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import java.nio.file.*
import java.util.concurrent.atomic.AtomicReference

/** Factory lifetime is independent of the UI coroutine. Cancellation settles
 * the producer, closes any undelivered owner, and only then releases admission. */
internal object DesktopMcpService {
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
    private val slot=Semaphore(1)
    suspend fun open(runtime:McpRuntime,privateParent:Path,captureHelper:Path,captureSha256:String,core:WorkbenchCore,deviceId:String):DesktopMcpCoordinator {
        if(!slot.tryAcquire())throw McpRefused()
        val owned=AtomicReference<DesktopMcpCoordinator?>(null)
        val producer=scope.async {
            var catalog:WorkbenchPackageCatalog?=null;var inbox:WorkbenchMcpInbox?=null
            var capture:McpCaptureEngine?=null;var connection:McpConnection?=null
            var stage=McpFailureStage.service_root
            try {
                val parent=withContext(Dispatchers.IO){privateRoot(privateParent)}
                stage=McpFailureStage.catalog
                val local=openPackageCatalog(parent.toString());catalog=local
                // No host exists yet, so no external reader can hold a package
                // from a previous-process pending retirement.
                local.pendingRetirement()?.let{local.retire(it.packageId,it.target,it.manifestSha256)}
                stage=McpFailureStage.inbox
                val returned=openMcpResultInbox(parent.toString());inbox=returned
                stage=McpFailureStage.capture_helper
                val captureOwner=NativeMcpCaptureEngine.open(captureHelper,captureSha256,core,deviceId,parent);capture=captureOwner
                val delegate=AtomicReference<McpOwnerActions?>(null)
                val actions=object:McpOwnerActions {
                    override suspend fun capture(session:String,selector:String)=requireNotNull(delegate.get()).capture(session,selector)
                    override suspend fun submitResult(session:String,packageId:String,manifestHash:String,text:String?,pngBase64:String?,note:String)=requireNotNull(delegate.get()).submitResult(session,packageId,manifestHash,text,pngBase64,note)
                }
                stage=McpFailureStage.host_ready
                val native=DesktopMcpHost.open(runtime,parent.resolve("service"),actions)
                val connected=NativeMcpConnection(native);connection=connected
                val result=DesktopMcpCoordinator(local,returned,connected,captureOwner)
                // The coordinator owns every resource before any suspending work.
                catalog=null;inbox=null;capture=null;connection=null;owned.set(result);delegate.set(result)
                stage=McpFailureStage.coordinator
                result.refresh();result
            } catch(error:CancellationException) {
                throw error
            } catch(_:Exception) {
                throw McpRefused(stage)
            } finally {
                withContext(NonCancellable){try{connection?.close()}finally{try{capture?.close()}finally{try{inbox?.close()}finally{catalog?.close()}}}}
            }
        }
        try {val result=producer.await();currentCoroutineContext().ensureActive();owned.set(null);return result}
        catch(error:Throwable){producer.cancel();withContext(NonCancellable){try{producer.await()}catch(_:Throwable){};owned.getAndSet(null)?.close()};throw error}
        finally{slot.release()}
    }
    private fun privateRoot(parent:Path):Path {
        require(parent.isAbsolute&&parent==parent.normalize()&&Files.isDirectory(parent,LinkOption.NOFOLLOW_LINKS))
        // Caller supplies the physical canonical app-private anchor retained by
        // DesktopNativeRuntime, not a client path or a project directory.
        require(!Files.isSymbolicLink(parent)&&parent.toRealPath()==parent)
        val root=parent.resolve("mcp-owner-v1")
        if(!Files.exists(root,LinkOption.NOFOLLOW_LINKS))Files.createDirectory(root)
        require(Files.isDirectory(root,LinkOption.NOFOLLOW_LINKS)&&!Files.isSymbolicLink(root)&&root.toRealPath()==root)
        val service=root.resolve("service")
        if(!Files.exists(service,LinkOption.NOFOLLOW_LINKS))Files.createDirectory(service)
        require(Files.isDirectory(service,LinkOption.NOFOLLOW_LINKS)&&!Files.isSymbolicLink(service)&&service.toRealPath()==service)
        return root
    }
}
