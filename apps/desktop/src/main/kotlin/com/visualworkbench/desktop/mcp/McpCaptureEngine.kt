package com.visualworkbench.desktop.mcp

import com.visualworkbench.bindings.host.*
import com.visualworkbench.desktop.capture.openDesktopCaptureService
import com.visualworkbench.desktop.capture.guardedMirrorCapture
import com.visualworkbench.desktop.capture.PickerReleaseAction
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
    private data class Source(val project:String,val document:String,val asset:String)
    private val sources=LinkedHashMap<Source,Path>()
    private fun source(info:PackageInfo)=Source(info.binding.projectId,info.binding.documentId,info.sourceAssetId)
    private val projectRetirement=McpCaptureProjectRetirement()
    private var retainedFrame:Pair<CapturedFrame,PickerReleaseAction>?=null
    private fun destroyFrame(value:CapturedFrame){
        val held=retainedFrame?:Pair(value,PickerReleaseAction{value.destroy()}).also{retainedFrame=it}
        check(held.first===value)
        held.second.close() // A failed generated destructor is sticky and retained.
        retainedFrame=null
    }
    private var closed=false
    private var released=false
    private val producers=CoroutineScope(SupervisorJob()+Dispatchers.Default)
    private suspend fun retainEditorTarget(target:CaptureTarget,receipt:CaptureFrameInfo):McpSelectedTarget=gate.withLock {
        check(!closed)
        require(receipt.borderVisible&&receipt.windowHandle==target.window)
        val exact=hostCall{service.refreshTarget(target)}
        require(exact.window==target.window&&exact.processId==target.processId&&exact.processCreated==target.processCreated)
        val existing=targets.entries.singleOrNull{it.value.window==target.window&&it.value.processId==target.processId&&it.value.processCreated==target.processCreated}
        val selector=existing?.key?:run{require(targets.size<16);UUID.randomUUID().toString().also{targets[it]=target}}
        McpSelectedTarget(selector,"Captured window | ${receipt.width} x ${receipt.height} px")
    }
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
    override suspend fun compile(selector:String,checkGrant:suspend()->Unit):WorkbenchCompiledPackage =
        compileVerification(selector,McpCaptureProfile(PackageTarget.Generic(),false),checkGrant)
    override suspend fun compileVerification(selector:String,profile:McpCaptureProfile,checkGrant:suspend()->Unit):WorkbenchCompiledPackage=gate.withLock {
        check(!closed);retainedFrame?.let{destroyFrame(it.first)};projectRetirement.settle();currentCoroutineContext().ensureActive()
        val selected=targets[selector]?:throw McpRefused();checkGrant()
        val fresh=hostCall{service.refreshTarget(selected)};checkGrant()
        val work=withContext(Dispatchers.IO){McpCaptureWork.create(root)}
        var frame:CapturedFrame?=null;var compiled:WorkbenchCompiledPackage?=null
        try {
            fun id()=core.newId(System.currentTimeMillis().toULong())
            val captured=hostCall(release={it:CapturedFrame->destroyFrame(it)}){operation->
                guardedMirrorCapture(TunnelSource.Window(fresh.window),{it:CapturedFrame->frame=it}){
                    service.capture(fresh,id(),work.path.toString(),operation)
                }
            };frame=captured
            val receipt=captured.info();require(receipt.borderVisible && receipt.windowHandle==selected.window)
            require(receipt.bitDepth==8.toUByte());checkGrant()
            withContext(Dispatchers.IO){work.record(receipt)}
            val projectId=id();val document=id();val destination=work.path.resolve("project.vwb")
            val opened=createCaptureProject(CreateFileProject(destination.toString(),projectId,document,id(),deviceId,"MCP window capture",work.path.resolve("capture.png").toString(),work.path.toString(),System.currentTimeMillis()),
                CaptureImportDescriptor(true,receipt.captureSessionId,receipt.frameId,receipt.geometryRevision,"windows","window",receipt.physicalX,receipt.physicalY,receipt.width,receipt.height,receipt.dpiScale,receipt.timestampNs,receipt.capturedAtMs,receipt.windowHandle,"",receipt.sourceAssetId)).also(projectRetirement::retain)
            checkGrant()
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
            val result=opened.compilePackage(PackageCompileOptions(WorkflowBinding(current.projectId,document,current.hostSeq,current.stateHash),id(),System.currentTimeMillis(),profile.target,snapshot,profile.includeWindowTitle))
            compiled=result;checkGrant();currentCoroutineContext().ensureActive()
            require(sources.size<32 || source(result.info) in sources)
            sources[source(result.info)]=destination
            // Keep the immutable original and canonical project for recovery,
            // including after revocation or a lost publication receipt.
            projectRetirement.settle()
            destroyFrame(captured);frame=null
            currentCoroutineContext().ensureActive();compiled=null;result
        } finally {
            withContext(NonCancellable){try{compiled?.close()}finally{try{projectRetirement.settle()}finally{frame?.let(::destroyFrame)}}}
        }
    }
    override suspend fun <T> withVerificationSources(original:PackageInfo,after:PackageInfo,body:suspend(WorkbenchProject,WorkbenchProject)->T):T = gate.withLock {
        check(!closed);projectRetirement.settle();currentCoroutineContext().ensureActive()
        val beforePath=sourcePath(original)
        val afterPath=sourcePath(after)
        projectRetirement.borrow(core,listOf(beforePath to original,afterPath to after)){body(it[0],it[1])}
    }
    override suspend fun <T> withVerificationSource(after:PackageInfo,body:suspend(WorkbenchProject)->T):T = gate.withLock {
        check(!closed);projectRetirement.settle();currentCoroutineContext().ensureActive()
        projectRetirement.borrow(core,listOf(sourcePath(after) to after)){body(it.single())}
    }
    /** Private source recovery only. This never reconstructs an OS selector or
     * grant. Existing native methods still verify source/capture/blob identity. */
    private suspend fun sourcePath(expected:PackageInfo):Path {
        sources[source(expected)]?.let{return it}
        require(sources.size<32)
        val excluded=sources.filterKeys{it!=source(expected)}.values.toSet()
        val found=findRetainedMcpCaptureProject(root,expected,core,projectRetirement,excluded)?:throw McpRefused()
        sources[source(expected)]=found
        return found
    }
    private suspend fun <T> hostCall(release:(T)->Unit={},body:suspend(CaptureOperation)->T):T {
        val operation=CaptureOperation();val owned=AtomicReference<T?>(null)
        val producer=producers.async{body(operation).also{owned.set(it)}}
        try {val value=producer.await();currentCoroutineContext().ensureActive();owned.set(null);return value}
        catch(error:CancellationException){operation.cancel();withContext(NonCancellable){try{producer.await()}catch(_:Exception){};owned.getAndSet(null)?.let(release)};throw error}
        finally{operation.destroy()}
    }
    override suspend fun close(){withContext(NonCancellable){gate.withLock{
        closed=true
        // A failed project close stays reachable and retryable here. Do not
        // release the capture/runtime owner before exact retirement succeeds.
        projectRetirement.settle()
        retainedFrame?.let{destroyFrame(it.first)}
        if(!released){targets.clear();sources.clear();service.destroy();producers.cancel();released=true}
    }}}
    companion object {
        /** The capture owner calls this with the same target, native receipt
         * and newly opened project it is delivering to the editor. Never create
         * it from an imported manifest, a window title or an agent request. */
        fun editorOrigin(target:CaptureTarget,receipt:CaptureFrameInfo,project:ProjectInfo,documentId:String):McpEditorCaptureOrigin {
            require(receipt.borderVisible&&receipt.windowHandle==target.window&&documentId in project.documentIds)
            val binding=WorkflowBinding(project.projectId,documentId,project.hostSeq,project.stateHash)
            return McpEditorCaptureOrigin(binding,receipt.sourceAssetId){engine->
                val native=engine as? NativeMcpCaptureEngine?:throw McpRefused()
                native.retainEditorTarget(target,receipt)
            }
        }
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
