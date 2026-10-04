package com.visualworkbench.desktop.mcp

import com.visualworkbench.bindings.host.*
import com.visualworkbench.desktop.capture.openDesktopCaptureService
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.*
import java.util.UUID
import java.util.concurrent.atomic.AtomicReference

/** Actual WGC/UIA adapter. Window identities are selected once through an
 * explicit temporary hotkey, then refreshed by exact HWND/PID/creation time.
 * A later foreground window is never used as a substitute. */
internal class NativeMcpCaptureEngine private constructor(
    private val service:CaptureService,private val core:WorkbenchCore,
    private val deviceId:String,private val root:Path,
):McpCaptureEngine {
    private val gate=Mutex()
    private val targets=LinkedHashMap<String,CaptureTarget>()
    private var closed=false
    private val producers=CoroutineScope(SupervisorJob()+Dispatchers.Default)
    override suspend fun select():McpSelectedTarget=gate.withLock {
        check(!closed);require(targets.size<16)
        // Owner activates another window and presses Ctrl+Alt+M. Native callback
        // samples the target before any application focus/activation occurs.
        val key=registerCaptureHotkey(3u,77u)
        try {
            val selected=withTimeout(15_000){var found:CaptureTarget?=null;while(found==null){currentCoroutineContext().ensureActive();found=key.take();if(found==null)delay(15)};requireNotNull(found)}
            val selector=UUID.randomUUID().toString()
            targets[selector]=selected
            McpSelectedTarget(selector,"Window ${targets.size} | ${selected.clientWidth} x ${selected.clientHeight} px")
        } finally {withContext(NonCancellable){try{key.shutdown()}finally{key.destroy()}}}
    }
    override suspend fun compile(selector:String,checkGrant:suspend()->Unit):WorkbenchCompiledPackage=gate.withLock {
        check(!closed);val selected=targets[selector]?:throw McpRefused();checkGrant()
        val fresh=hostCall{service.refreshTarget(selected)};checkGrant()
        val work=withContext(Dispatchers.IO){McpCaptureWork.create(root)}
        var frame:CapturedFrame?=null;var project:WorkbenchProject?=null;var compiled:WorkbenchCompiledPackage?=null
        try {
            fun id()=core.newId(System.currentTimeMillis().toULong())
            val captured=hostCall(release={it:CapturedFrame->it.destroy()}){operation->service.capture(fresh,id(),work.path.toString(),operation)};frame=captured
            val receipt=captured.info();require(receipt.borderVisible && receipt.windowHandle==selected.window)
            require(receipt.bitDepth==8.toUByte());checkGrant()
            withContext(Dispatchers.IO){work.record(receipt)}
            val projectId=id();val document=id();val destination=work.path.resolve("project.vwb")
            val opened=createCaptureProject(CreateFileProject(destination.toString(),projectId,document,id(),deviceId,"MCP window capture",work.path.resolve("capture.png").toString(),work.path.toString(),System.currentTimeMillis()),
                CaptureImportDescriptor(true,receipt.captureSessionId,receipt.frameId,receipt.geometryRevision,"windows","window",receipt.physicalX,receipt.physicalY,receipt.width,receipt.height,receipt.dpiScale,receipt.timestampNs,receipt.capturedAtMs,receipt.windowHandle,"",receipt.sourceAssetId))
            project=opened;checkGrant()
            val original=opened.info()
            val ticket=semanticWorkflows(opened).beginCapture(WorkflowBinding(original.projectId,document,original.hostSeq,original.stateHash))
            var snapshot:String?=null
            try {
                val tree=hostCall{operation->captured.collect(operation)};checkGrant()
                require(tree.sourceAssetId==ticket.describe().sourceAssetId)
                val elements=tree.elements.map{e->CapturedSemanticElement(e.localId,e.parentLocalId,e.name,e.role,e.automationId,e.resourceId,e.htmlId,Rect(e.x,e.y,e.width,e.height),e.text,e.enabled,e.focused)}
                val next=id()
                val plan=ticket.prepare(WorkflowMetadata(id(),deviceId,original.nextLamport,System.currentTimeMillis()),next,
                    if(tree.platform=="chromium_uia")SemanticPlatform.ChromiumUia else SemanticPlatform.Uia,
                    tree.frameDeltaMs,tree.collectionElapsedMs,SemanticBoundsSpace.HostPhysical,elements)
                try{plan.commit();snapshot=next}finally{plan.close()}
            } catch(error:CancellationException){throw error}
            catch(_:Exception){withContext(Dispatchers.IO){work.semanticUnavailable()}}
            finally{withContext(NonCancellable){ticket.close()}}
            checkGrant();val current=opened.info()
            val result=opened.compilePackage(PackageCompileOptions(WorkflowBinding(current.projectId,document,current.hostSeq,current.stateHash),id(),System.currentTimeMillis(),PackageTarget.Generic(),snapshot))
            compiled=result;checkGrant();currentCoroutineContext().ensureActive()
            // Keep the immutable original and canonical project for recovery,
            // including after revocation or a lost publication receipt.
            withContext(NonCancellable){opened.close()};project=null
            captured.destroy();frame=null
            currentCoroutineContext().ensureActive();compiled=null;result
        } finally {
            withContext(NonCancellable){try{compiled?.close()}finally{try{project?.close()}finally{frame?.destroy()}}}
        }
    }
    private suspend fun <T> hostCall(release:(T)->Unit={},body:suspend(CaptureOperation)->T):T {
        val operation=CaptureOperation();val owned=AtomicReference<T?>(null)
        val producer=producers.async{body(operation).also{owned.set(it)}}
        try {val value=producer.await();currentCoroutineContext().ensureActive();owned.set(null);return value}
        catch(error:CancellationException){operation.cancel();withContext(NonCancellable){try{producer.await()}catch(_:Exception){};owned.getAndSet(null)?.let(release)};throw error}
        finally{operation.destroy()}
    }
    override suspend fun close(){withContext(NonCancellable){gate.withLock{if(!closed){closed=true;targets.clear();service.destroy();producers.cancel()}}}}
    companion object {
        suspend fun open(helper:Path,expectedSha256:String,core:WorkbenchCore,deviceId:String,privateParent:Path):NativeMcpCaptureEngine {
            val captureRoot=withContext(Dispatchers.IO){McpCaptureWork.prepare(privateParent)}
            val native=openDesktopCaptureService(helper,expectedSha256)
            return try{NativeMcpCaptureEngine(native,core,deviceId,captureRoot)}catch(error:Throwable){native.destroy();throw error}
        }
    }
}

/** This dedicated inbox retains successful/interrupted originals. There is no
 * recursive cleanup or expiry: 32 captures / 512MiB admission includes all
 * retained project databases, blobs and partial files before reserving160MiB.
 * Owner cleanup is an explicit future action, never a side effect of shutdown. */
private class McpCaptureWork private constructor(val path:Path) {
    fun record(receipt:CaptureFrameInfo){write(path.resolve("recovery.txt"),"Visual Workbench MCP capture original\nFile: capture.png\nSource BLAKE3: ${receipt.sourceAssetId}\nExtent: ${receipt.width} x ${receipt.height}\nCanonical project: project.vwb (if published)\n")}
    fun semanticUnavailable(){write(path.resolve("semantic-unavailable.txt"),"This capture has no accepted accessibility snapshot. No element identity was invented.\n")}
    companion object {
        fun prepare(parent:Path):Path {
            require(parent.isAbsolute&&Files.isDirectory(parent,LinkOption.NOFOLLOW_LINKS));plain(parent)
            val root=parent.resolve("mcp-captures-v1")
            if(!Files.exists(root,LinkOption.NOFOLLOW_LINKS))Files.createDirectory(root)
            plain(root);return root
        }
        fun create(root:Path):McpCaptureWork {
            plain(root);var entries=0;var count=0;var bytes=0L
            Files.list(root).use{children->children.forEach{require(++entries<=31)}}
            Files.walk(root,16).use{paths->paths.forEach{path->
                require(++count<=8192);plain(path)
                if(Files.isRegularFile(path,LinkOption.NOFOLLOW_LINKS))bytes=Math.addExact(bytes,Files.size(path))
                else require(Files.isDirectory(path,LinkOption.NOFOLLOW_LINKS)&&root.relativize(path).nameCount<16)
                require(bytes<=512L*1024*1024-160L*1024*1024)
            }}
            val path=Files.createDirectory(root.resolve(UUID.randomUUID().toString()))
            write(path.resolve("owner-v1"),"Visual Workbench private MCP capture v1\n")
            return McpCaptureWork(path)
        }
        private fun plain(path:Path){require(path.isAbsolute);var at=path.root;for(part in path){at=at.resolve(part);require(!Files.isSymbolicLink(at)&&at.toRealPath()==at.toAbsolutePath().normalize())}}
        private fun write(path:Path,value:String){val bytes=value.toByteArray(Charsets.UTF_8);require(bytes.size<=2048);FileChannel.open(path,StandardOpenOption.CREATE_NEW,StandardOpenOption.WRITE).use{file->val buffer=ByteBuffer.wrap(bytes);while(buffer.hasRemaining())file.write(buffer);file.force(true)}}
    }
}
