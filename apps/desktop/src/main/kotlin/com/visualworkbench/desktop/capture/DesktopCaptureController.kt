package com.visualworkbench.desktop.capture

import com.visualworkbench.bindings.host.*
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.StandardOpenOption
import java.nio.channels.FileChannel
import java.nio.ByteBuffer
import java.util.UUID
import java.util.concurrent.atomic.AtomicReference

internal data class ForegroundCaptureState(
    val hotkeyEnabled: Boolean = false, val busy: Boolean = false,
    val cleanupPending: Boolean = false, val stopping: Boolean = false,
)

/** No global hook is registered by construction. The owner explicitly enables
 * it after choosing the key; the target HWND is sampled in the native hotkey
 * callback before the app gets focus. Capturing an app-owned HWND is refused. */
internal class DesktopCaptureController(
    private val scope:CoroutineScope,private val core:WorkbenchCore,private val service:CaptureService,
    private val projects:Path,private val deviceId:String,
    private val route:()->(suspend(WorkbenchProject,Path,String?)->Unit),private val report:(String)->Unit,
    private val mayCapture: () -> Boolean = { true },
) {
    private val mutable = MutableStateFlow(ForegroundCaptureState())
    val state = mutable.asStateFlow()
    private val admission=CaptureAdmission(scope) { value -> mutable.update { it.copy(busy = value) } }
    private var key:CaptureHotkey?=null;private var polling:Job?=null;private var closed=false
    private var keyRelease: PickerReleaseAction? = null
    private val serviceRelease = PickerReleaseAction { service.destroy() }
    private var cleanupFailure: FailedCleanup? = null
    private class FailedCleanup(val error: Throwable, val retained: List<Any>)
    private val producers=CoroutineScope(SupervisorJob()+Dispatchers.Default)
    fun enableHotkey(modifiers:UInt=3u,keyCode:UInt=65u){
        if (closed || mutable.value.cleanupPending || !scope.isActive) return
        if (polling?.isCompleted == false) return
        try{releaseHotkey();val installed=registerCaptureHotkey(modifiers,keyCode);key=installed
            keyRelease=PickerReleaseAction { installed.shutdown(); installed.destroy() }
            mutable.update { it.copy(hotkeyEnabled = true) };polling=scope.launch{try{while(isActive){
            val target=key?.take();if(target!=null)start(target);delay(15)
        }}catch(error:CancellationException){throw error}catch(_:Exception){disableHotkey();report("The capture hotkey stopped. Review its cleanup status before retrying.")}}}
        catch(error:CancellationException){throw error}
        catch(_:Exception){report(if(mutable.value.cleanupPending)"Foreground capture cleanup is pending. Its owners remain held." else "The capture hotkey is unavailable. Choose another key; no hook was enabled.")}
    }
    fun disableHotkey(){try{releaseHotkey()}catch(_:Exception){report("Foreground capture cleanup is pending. Its owners remain held.")}}
    private fun releaseHotkey(){
        // Keep the cancelled poll owner until shutdown joins it. Enabling a
        // replacement waits for completion rather than discarding that job.
        polling?.cancel()
        if(key==null)return
        try{checkNotNull(keyRelease).close();key=null;keyRelease=null;mutable.update{it.copy(hotkeyEnabled=false)}}
        catch(error:Exception){mutable.update{it.copy(cleanupPending=true)};throw error}
    }
    private fun start(target:CaptureTarget){
        if(closed || mutable.value.cleanupPending || !mayCapture())return
        val deliver = route() // Capture project/link/revision before producing the screenshot.
        if(!admission.start {
            var project:WorkbenchProject?=null;var frame:CapturedFrame?=null;var workspace:Workspace?=null
            val lateFrame=AtomicReference<CapturedFrame?>(null)
            try{
                val work=Workspace.create(projects.resolve(".capture-inbox"));workspace=work
                val now=System.currentTimeMillis();fun id()=core.newId(now.toULong())
                val captured=hostCall(release={it:CapturedFrame->lateFrame.set(it)}){operation->service.capture(target,id(),work.path.toString(),operation)};frame=captured
                val receipt=captured.info();work.record(receipt)
                val projectId=id();val document=id();val destination=projects.resolve("$projectId.vwb")
                val opened=createCaptureProject(CreateFileProject(destination.toString(),projectId,document,id(),deviceId,"Screen capture",work.path.resolve("capture.png").toString(),work.path.toString(),System.currentTimeMillis()),
                    CaptureImportDescriptor(true,receipt.captureSessionId,receipt.frameId,receipt.geometryRevision,"windows","window",receipt.physicalX,receipt.physicalY,
                        receipt.width,receipt.height,receipt.dpiScale,receipt.timestampNs,receipt.capturedAtMs,receipt.windowHandle,"",receipt.sourceAssetId));project=opened
                val info=opened.info();val ticket=semanticWorkflows(opened).beginCapture(WorkflowBinding(info.projectId,document,info.hostSeq,info.stateHash))
                var warning:String?=null
                try{
                    val tree=hostCall{operation->captured.collect(operation)}
                    require(tree.sourceAssetId==ticket.describe().sourceAssetId)
                    val elements=tree.elements.map{e->CapturedSemanticElement(e.localId,e.parentLocalId,e.name,e.role,e.automationId,e.resourceId,e.htmlId,
                        Rect(e.x,e.y,e.width,e.height),e.text,e.enabled,e.focused)}
                    val plan=ticket.prepare(WorkflowMetadata(id(),deviceId,info.nextLamport,System.currentTimeMillis()),id(),
                        if(tree.platform=="chromium_uia")SemanticPlatform.ChromiumUia else SemanticPlatform.Uia,
                        tree.frameDeltaMs,tree.collectionElapsedMs,SemanticBoundsSpace.HostPhysical,elements)
                    try{plan.commit()}finally{plan.close()}
                }catch(error:CancellationException){throw error}
                catch(_:Exception){warning="Screenshot saved; accessibility information was unavailable or the source changed."}
                finally{ticket.close()}
                work.release();workspace=null
                // The existing destination owns close at invocation, including
                // route refusal/cancellation. Do not attempt a second close.
                project=null
                deliverCaptureProject(opened,{deliver(it,destination,warning)}){held,error->
                    retainCleanupFailure(error,listOf(held))
                }
            }catch(_:CancellationException){report("Capture cancelled. Any published project or completed original remains saved.")}
            catch(_:Exception){report("Capture was refused or could not finish. ${workspace?.recoveryMessage().orEmpty()}")}
            finally{withContext(NonCancellable){
                val heldFrame=frame?:lateFrame.get()
                try{project?.close();heldFrame?.destroy();workspace?.removeIfEmpty()}
                catch(error:Exception){
                    retainCleanupFailure(error,listOfNotNull(project,heldFrame,workspace))
                }
            }}
        })report("A capture is already running or its owner is stopping.")
    }
    private fun retainCleanupFailure(error:Throwable,owners:List<Any>){
        val prior=cleanupFailure
        if(prior==null)cleanupFailure=FailedCleanup(error,owners)
        else{
            if(prior.error!==error)prior.error.addSuppressed(error)
            cleanupFailure=FailedCleanup(prior.error,prior.retained+owners)
        }
        mutable.update{it.copy(cleanupPending=true)}
        report("Foreground capture cleanup is pending. Native and private file owners remain held.")
    }
    fun cancel(){admission.cancel()}
    suspend fun shutdown(){
        closed=true;mutable.update{it.copy(stopping=true)}
        withContext(NonCancellable){
            // Always signal/join capture before reporting another held owner.
            admission.shutdown()
            polling?.cancelAndJoin();polling=null
            cleanupFailure?.let{throw it.error}
            releaseHotkey()
            try{serviceRelease.close()}catch(error:Exception){mutable.update{it.copy(cleanupPending=true)};throw error}
            producers.cancel()
        }
    }
    private suspend fun <T> hostCall(release:(T)->Unit={},body:suspend(CaptureOperation)->T):T {
        currentCoroutineContext().ensureActive()
        val operation=CaptureOperation()
        try { return settledPicker(producers, release, { operation.cancel() }) { body(operation) } }
        finally { operation.destroy() }
    }
}
/** Opening a hash-pinned native service also has an owned-result handoff; a
 * cancelled caller cannot strand the helper's read lock in a discarded handle. */
internal suspend fun openDesktopCaptureService(helper:Path,packagedSha256:String):CaptureService {
    if(!CaptureServiceFactory.slot.tryAcquire())throw IllegalStateException("Capture startup is busy")
    val owned=AtomicReference<CaptureService?>(null)
    try{
        val producer=CaptureServiceFactory.scope.async{createCaptureService(helper.toString(),packagedSha256).also{owned.set(it)}}
        try{val result=producer.await();currentCoroutineContext().ensureActive();owned.set(null);return result}
        catch(error:CancellationException){withContext(NonCancellable){try{producer.await()}catch(_:Exception){};owned.getAndSet(null)?.destroy()};throw error}
    }finally{CaptureServiceFactory.slot.release()}
}
private object CaptureServiceFactory {
    val slot=kotlinx.coroutines.sync.Semaphore(1)
    val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
}
private class Workspace private constructor(val path:Path,private val token:String){
    private val marker get()=path.resolve("owner-v1")
    fun record(value:CaptureFrameInfo){
        // Numeric/source provenance only, never window title or accessibility
        // text. This lets an owner recover the original after interrupted import.
        write(path.resolve("recovery.txt"),"Visual Workbench private capture\nOriginal: capture.png\nSource BLAKE3: ${value.sourceAssetId}\nExtent: ${value.width} x ${value.height}\nCaptured at: ${value.capturedAtMs}\nCopy capture.png to retry ordinary image import.\n")
    }
    fun recoveryMessage():String=if(Files.isRegularFile(path.resolve("capture.png"),LinkOption.NOFOLLOW_LINKS))"The original remains at $path. See recovery.txt when present." else ""
    fun removeIfEmpty(){if(!Files.exists(path.resolve("capture.png"),LinkOption.NOFOLLOW_LINKS))release()}
    fun release(){require(Files.isRegularFile(marker,LinkOption.NOFOLLOW_LINKS)&&Files.size(marker)==token.length.toLong()&&Files.readString(marker)==token)
        val entries=Files.list(path).use{it.toList()};require(entries.all{it.fileName.toString() in setOf("owner-v1","capture.png","recovery.txt")&&Files.isRegularFile(it,LinkOption.NOFOLLOW_LINKS)})
        entries.filter{it!=marker}.forEach{Files.delete(it)};Files.delete(marker);Files.delete(path)
    }
    companion object{
        fun create(root:Path):Workspace{
            require(root.isAbsolute&&root==root.normalize())
            val parent=root.parent;require(parent!=null&&parent.toRealPath()==parent.toAbsolutePath().normalize())
            if(!Files.exists(root,LinkOption.NOFOLLOW_LINKS))Files.createDirectory(root)
            require(Files.isDirectory(root,LinkOption.NOFOLLOW_LINKS)&&root.toRealPath()==root.toAbsolutePath().normalize())
            require(Files.list(root).use{it.limit(65).count()}<64)
            val path=Files.createDirectory(root.resolve(UUID.randomUUID().toString()));val token=UUID.randomUUID().toString()
            try{write(path.resolve("owner-v1"),token);return Workspace(path,token)}catch(error:Throwable){Files.deleteIfExists(path.resolve("owner-v1"));Files.deleteIfExists(path);throw error}
        }
        private fun write(path:Path,text:String){val bytes=text.toByteArray();require(bytes.size<=2048);FileChannel.open(path,StandardOpenOption.CREATE_NEW,StandardOpenOption.WRITE).use{val buffer=ByteBuffer.wrap(bytes);while(buffer.hasRemaining())it.write(buffer);it.force(true)}}
    }
}
