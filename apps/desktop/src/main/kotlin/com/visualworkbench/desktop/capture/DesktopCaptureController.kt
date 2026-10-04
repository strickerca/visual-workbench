package com.visualworkbench.desktop.capture

import com.visualworkbench.bindings.host.*
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.StandardOpenOption
import java.nio.channels.FileChannel
import java.nio.ByteBuffer
import java.util.UUID
import java.util.concurrent.atomic.AtomicReference

/** No global hook is registered by construction. The owner explicitly enables
 * it after choosing the key; the target HWND is sampled in the native hotkey
 * callback before the app gets focus. Capturing an app-owned HWND is refused. */
internal class DesktopCaptureController(
    private val scope:CoroutineScope,private val core:WorkbenchCore,private val service:CaptureService,
    private val projects:Path,private val deviceId:String,
    private val route:()->(suspend(WorkbenchProject,Path,String?)->Unit),private val report:(String)->Unit,
) {
    private val admission=CaptureAdmission(scope);private var key:CaptureHotkey?=null;private var polling:Job?=null;private var closed=false
    val hotkeyEnabled=kotlinx.coroutines.flow.MutableStateFlow(false)
    private val producers=CoroutineScope(SupervisorJob()+Dispatchers.Default)
    fun enableHotkey(modifiers:UInt=3u,keyCode:UInt=65u){
        check(!closed);disableHotkey()
        try{key=registerCaptureHotkey(modifiers,keyCode);hotkeyEnabled.value=true;polling=scope.launch{try{while(isActive){
            val target=key?.take();if(target!=null)start(target);delay(15)
        }}catch(error:CancellationException){throw error}catch(_:Exception){disableHotkey();report("The capture hotkey stopped. Enable it again to retry.")}}}catch(_:Exception){report("The capture hotkey is unavailable. Choose another key; no hook was enabled.")}
    }
    fun disableHotkey(){polling?.cancel();polling=null;try{key?.let{try{it.shutdown()}finally{it.destroy()}}}finally{key=null;hotkeyEnabled.value=false}}
    private fun start(target:CaptureTarget){
        if(closed)return
        val deliver = route() // Capture project/link/revision before producing the screenshot.
        if(!admission.start {
            var project:WorkbenchProject?=null;var frame:CapturedFrame?=null;var workspace:Workspace?=null;var transferred=false
            try{
                val work=Workspace.create(projects.resolve(".capture-inbox"));workspace=work
                val now=System.currentTimeMillis();fun id()=core.newId(now.toULong())
                val captured=hostCall(release={it:CapturedFrame->it.destroy()}){operation->service.capture(target,id(),work.path.toString(),operation)};frame=captured
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
                deliver(opened,destination,warning);transferred=true
            }catch(_:CancellationException){report("Capture cancelled. Any published project or completed original remains saved.")}
            catch(_:Exception){report("Capture was refused or could not finish. ${workspace?.recoveryMessage().orEmpty()}")}
            finally{try{withContext(NonCancellable){try{if(!transferred)project?.close()}finally{frame?.destroy()}}}
                finally{workspace?.removeIfEmpty()}}
        })report("A capture is already running or its owner is stopping.")
    }
    fun cancel(){admission.cancel()}
    suspend fun shutdown(){closed=true;try{disableHotkey()}finally{try{admission.shutdown()}finally{service.destroy();producers.cancel()}}}
    private suspend fun <T> hostCall(release:(T)->Unit={},body:suspend(CaptureOperation)->T):T {
        val operation=CaptureOperation();val owned=AtomicReference<T?>(null)
        try{val producer=producers.async{body(operation).also{owned.set(it)}}
            try{val value=producer.await();currentCoroutineContext().ensureActive();owned.set(null);return value}
            catch(error:CancellationException){operation.cancel();withContext(NonCancellable){try{producer.await()}catch(_:Exception){};owned.getAndSet(null)?.let(release)};throw error}
        }finally{operation.destroy()}
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
    fun removeIfEmpty(){if(!Files.exists(path.resolve("capture.png"),LinkOption.NOFOLLOW_LINKS))runCatching{release()}}
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
