package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlin.math.*

public data class DocumentAttachment(public val project: WorkbenchProject, public val snapshot: DocumentSnapshot, public val epoch: Long)
public data class DocumentState(public val descriptor: EditableDescriptor? = null, public val page: UInt = 0u,
    public val busy: Boolean = false, public val textTool: Boolean = false, public val regionTool: Boolean = false,
    public val glyphs: List<DocumentGlyph> = emptyList(), public val regions: List<DocumentRegion> = emptyList(),
    public val notices: List<String> = emptyList(), public val message: String? = null,
    public val export: DocumentExport? = null, public val passwordNeeded: Boolean = false,
    public val frame: DocumentDisplayFrame? = null) {
    public val tiles: List<DocumentDisplayTile> get() = frame?.tiles.orEmpty()
}
/** UI-dispatcher owner. Every adoption checks source, page, full visible revision,
 * attachment epoch and monotonically increasing operation/viewport generations. */
public class DocumentController(private val scope: CoroutineScope, private val core: WorkbenchCore, private val jobs: DocumentJobs,
    private val attachment: () -> DocumentAttachment?, private val admitted: () -> Boolean,
    private val changePage: suspend (DocumentAttachment, UInt) -> Unit) {
    private val mutable: MutableStateFlow<DocumentState> = MutableStateFlow(DocumentState())
    public val state: StateFlow<DocumentState> = mutable
    private val operationGate:Mutex=Mutex()
    private val tasks: MutableSet<Job> = mutableSetOf()
    private var command: Job? = null
    private var tileTask: Job? = null
    private var generation: Long = 0
    private var viewportGeneration: Long = 0
    private var viewport: Camera? = null
    private var password: String? = null
    private var epoch: Long? = null
    private var noticesPending: Boolean = false
    private var noticesTask: Job? = null
    public val pcRedaction: Boolean get() = jobs.pcRedaction
    /** SVG's portable final-byte carrier is verified. PDFium/qpdf final-file
     * contracts remain separate and do not authorize PDF publication yet. */
    public fun canExport(mode:DocumentCopy):Boolean = state.value.descriptor?.kind==EditableKind.Svg && mode==DocumentCopy.SvgAnnotations
    public val exportPublicationAvailable:Boolean get() = canExport(DocumentCopy.SvgAnnotations)
    public fun copyUnavailableText(mode:DocumentCopy):String = if(mode==DocumentCopy.Original)
        "Exact original PDF copies cannot add the required revision and source metadata."
        else "PDF copies are unavailable in this build. Your original is preserved."
    public fun bind(descriptor: EditableDescriptor?, page: UInt): Unit {
        val currentEpoch = attachment()?.epoch
        val old = state.value
        if (old.descriptor == descriptor && old.page == page && epoch == currentEpoch) return
        val sameSource = descriptor != null && old.descriptor?.sourceAssetId == descriptor.sourceAssetId &&
            old.descriptor.documentId == descriptor.documentId && old.descriptor.kind == descriptor.kind &&
            old.descriptor.pages == descriptor.pages && epoch == currentEpoch
        generation++;viewportGeneration++;viewport = null;tasks.toList().forEach { it.cancel() }
        command = null;tileTask = null;noticesTask?.cancel();noticesTask=null;epoch = currentEpoch
        if (!sameSource) noticesPending = descriptor != null
        if (!sameSource) password = null
        mutable.value = DocumentState(descriptor,page,regions=if(sameSource)old.regions else emptyList(),notices=if(sameSource)old.notices else emptyList())
    }
    /** App owners call after runtime installation, including import and reopen.
     * Wait for editor admission without borrowing source bytes or a worker. */
    public fun scheduleNotices(): Unit {
        if (!noticesPending || noticesTask?.isActive == true || state.value.descriptor == null) return
        val task = scope.launch(start=CoroutineStart.LAZY) {
            while (!admitted() || state.value.busy) delay(25)
            currentCoroutineContext().ensureActive()
            refreshNotices()
        }
        noticesTask=task;tasks+=task
        task.invokeOnCompletion { tasks.remove(task);if(noticesTask===task)noticesTask=null }
        task.start()
    }
    private fun capture(): Pair<DocumentAttachment,EditableDescriptor> {
        val a=attachment() ?: throw DocumentFailure(DocumentFailureKind.Stale)
        val d=state.value.descriptor ?: throw DocumentFailure(DocumentFailureKind.Unavailable)
        if(a.epoch != epoch || a.snapshot.documentId != d.documentId || a.snapshot.page?.let{it!=state.value.page}==true || !a.snapshot.render.revision.sameDocumentRevision(d.revision))
            throw DocumentFailure(DocumentFailureKind.Stale)
        return a to d
    }
    private fun current(a: DocumentAttachment,d: EditableDescriptor,ticket: Long,page: UInt): Boolean {
        val now=attachment() ?: return false
        return ticket==generation && now.project===a.project && now.epoch==a.epoch && state.value.page==page &&
            state.value.descriptor?.sourceAssetId==d.sourceAssetId && now.snapshot.documentId==d.documentId && now.snapshot.page?.let{it==page}!=false && now.snapshot.render.revision.sameDocumentRevision(d.revision)
    }
    private fun launchOwned(commandTask:Boolean=false,tile:Boolean=false,block: suspend CoroutineScope.() -> Unit): Job {
        val task=scope.launch(start=CoroutineStart.LAZY){operationGate.withLock{block()}}
        if(commandTask)command=task
        if(tile)tileTask=task
        tasks+=task;task.invokeOnCompletion { tasks.remove(task);if(tileTask===task)tileTask=null;if(command===task)command=null };task.start();return task
    }
    private fun work(block: suspend (DocumentAttachment,EditableDescriptor,Long,UInt) -> Unit): Unit {
        if(state.value.busy)return
        if(!admitted()){mutable.value=state.value.copy(message="Finish the current edit or dialog before document work.");return}
        val (a,d)=try { capture() } catch(error:DocumentFailure) { mutable.value=state.value.copy(message=documentFailureText(error));return }
        val ticket=++generation;val page=state.value.page
        tileTask?.cancel();tileTask=null;viewportGeneration++;viewport=null
        mutable.value=state.value.copy(busy=true,message=null,export=null)
        launchOwned(commandTask=true){
            try { if(!admitted())throw DocumentFailure(DocumentFailureKind.Busy);block(a,d,ticket,page) }
            catch(cancel:CancellationException) { throw cancel }
            catch(error:DocumentFailure) { if(current(a,d,ticket,page))mutable.value=state.value.copy(message=documentFailureText(error),passwordNeeded=error.kind==DocumentFailureKind.Password) }
            catch(error:CoreFailure) { if(current(a,d,ticket,page))mutable.value=state.value.copy(message=documentFailureText(DocumentFailure(when(error.kind){
                CoreFailureKind.Unsupported->DocumentFailureKind.Unsupported
                CoreFailureKind.Invalid->DocumentFailureKind.Stale
                CoreFailureKind.Cancelled->DocumentFailureKind.Cancelled
                CoreFailureKind.Backpressure->DocumentFailureKind.Limit
                CoreFailureKind.Closed->DocumentFailureKind.Unavailable
                else->DocumentFailureKind.Worker
            }))) }
            catch(_:Exception) { if(current(a,d,ticket,page))mutable.value=state.value.copy(message="Document work could not finish. The original is preserved.") }
            finally { if(ticket==generation) mutable.value=state.value.copy(busy=false) }
        }
    }
    private fun password(a:DocumentAttachment,d:EditableDescriptor):String? = password ?: (a.project as? WorkbenchDocumentsProject)?.documentPassword(d.documentId)
    public fun refreshNotices(): Unit = work { a,d,ticket,page ->
        val source=(a.project as WorkbenchDocumentsProject).documentSource(d.documentId,d.revision)
        if(source.descriptor!=d) throw DocumentFailure(DocumentFailureKind.Verification)
        val reply=jobs.run(DocumentRequest(if(d.kind==EditableKind.Pdf)DocumentOperation.PdfInfo else DocumentOperation.SvgInfo,password(a,d)),source.bytes)
        val info=reply.metadata as? DocumentMetadata.Info ?: throw DocumentFailure(DocumentFailureKind.Verification)
        if(reply.sourceBlake3!=d.sourceAssetId || info.kind!=d.kind || info.pages!=d.pages.size.toUInt()) throw DocumentFailure(DocumentFailureKind.Verification)
        if(info.notices.size>64 || info.notices.any{it.length>64}) throw DocumentFailure(DocumentFailureKind.Verification)
        if(current(a,d,ticket,page)) { noticesPending=false;mutable.value=state.value.copy(notices=info.notices.toList(),passwordNeeded=false) }
    }
    public fun password(value:String):Unit {
        if(state.value.busy || value.length>1024 || '\u0000' in value || !admitted())return
        val (a,d)=try{capture()}catch(_:DocumentFailure){return}
        (a.project as? WorkbenchDocumentsProject)?.documentPassword(d.documentId,value)
        password=value;viewport=null;refreshNotices()
    }
    public fun page(index:UInt):Unit = work { a,d,ticket,page ->
        if(index>=d.pages.size.toUInt())throw DocumentFailure(DocumentFailureKind.Invalid)
        if(!current(a,d,ticket,page))throw DocumentFailure(DocumentFailureKind.Stale)
        changePage(a,index)
        currentCoroutineContext().ensureActive()
        val now=attachment()
        if(ticket==generation && now?.project===a.project && now.epoch==a.epoch && now.snapshot.documentId==d.documentId) {
            val updated=(a.project as WorkbenchDocumentsProject).documentDescriptor(d.documentId) ?: throw DocumentFailure(DocumentFailureKind.Stale)
            if(now.snapshot.render.revision.sameDocumentRevision(updated.revision)) {
                viewportGeneration++;viewport=null
                mutable.value=state.value.copy(descriptor=updated,page=index,busy=false,glyphs=emptyList(),export=null,frame=null,textTool=false,regionTool=false)
            }
        }
    }
    public fun textTool(enabled:Boolean):Unit { if(!state.value.busy&&admitted()&&state.value.descriptor?.kind==EditableKind.Pdf)mutable.value=state.value.copy(textTool=enabled,regionTool=false,glyphs=emptyList()) }
    public fun regionTool(enabled:Boolean):Unit { if(!state.value.busy&&admitted()&&state.value.descriptor?.kind==EditableKind.Pdf)mutable.value=state.value.copy(regionTool=enabled,textTool=false,glyphs=emptyList()) }
    public fun select(rectangle:Rect):Unit = work { a,d,ticket,page ->
        val source=(a.project as WorkbenchDocumentsProject).documentSource(d.documentId,d.revision)
        val glyphs=DocumentInteraction(jobs,core).select(source,page,rectangle,password(a,d))
        if(current(a,d,ticket,page))mutable.value=state.value.copy(glyphs=glyphs)
    }
    public fun region(rectangle:Rect):Unit {
        if(!admitted()||state.value.busy||state.value.descriptor?.kind!=EditableKind.Pdf||state.value.regions.size>=4096)return
        if(!listOf(rectangle.x,rectangle.y,rectangle.width,rectangle.height).all{it.isFinite()}||rectangle.width<=0||rectangle.height<=0)return
        mutable.value=state.value.copy(regions=state.value.regions+DocumentRegion(state.value.page,rectangle),export=null)
    }
    public fun redactSelection():Unit {
        val boxes=state.value.glyphs.mapNotNull{it.bounds};if(boxes.isEmpty())return
        val x=boxes.minOf{it.x};val y=boxes.minOf{it.y}
        region(Rect(x,y,boxes.maxOf{it.x+it.width}-x,boxes.maxOf{it.y+it.height}-y))
    }
    public fun clearRegions():Unit { if(!state.value.busy&&admitted())mutable.value=state.value.copy(regions=emptyList(),export=null) }
    public fun export(mode:DocumentCopy,removeEncryption:Boolean=false):Unit {
        if(!canExport(mode)){mutable.value=state.value.copy(message=copyUnavailableText(mode));return}
        prepareExport(mode,removeEncryption)
    }
    private fun prepareExport(mode:DocumentCopy,removeEncryption:Boolean):Unit = work { a,d,ticket,page ->
        val result=DocumentInteraction(jobs,core).export(a.project,d,mode,state.value.regions,password(a,d),removeEncryption)
        if(current(a,d,ticket,page))mutable.value=state.value.copy(export=result,notices=result.notices,message=if(result.redacted)"Verified redaction copy is ready." else "Document copy is ready; this is not a redaction export.")
    }
    public fun checkedExport():DocumentExport? {
        val result=state.value.export ?: return null
        val (a,d)=try{capture()}catch(_:DocumentFailure){return null}
        return result.takeIf{canExport(it.mode)&&it.exportIdentity==DocumentExportIdentity(1u,d.revision.hostSeq,d.revision.stateHash,d.sourceAssetId)&&it.sourceAssetId==d.sourceAssetId&&a.snapshot.render.revision.sameDocumentRevision(it.revision)&&!state.value.busy}
    }
    public fun viewport(camera:Camera):Unit {
        if(state.value.descriptor==null||state.value.busy||!admitted()||camera.viewportWidth<=1||camera.viewportHeight<=1||viewport==camera)return
        val (a,d)=try{capture()}catch(_:DocumentFailure){return};val page=state.value.page
        val ticket=generation;val viewTicket=++viewportGeneration;viewport=camera
        val previous=tileTask;previous?.cancel()
        mutable.value=state.value.copy(frame=null)
        launchOwned(tile=true){
            previous?.join()
            try {
                val geometry=d.pages.getOrNull(page.toInt()) ?: throw DocumentFailure(DocumentFailureKind.Invalid)
                val corners=core.mapPoints(camera,true,listOf(Point(0.0,0.0),Point(camera.viewportWidth,0.0),Point(0.0,camera.viewportHeight),Point(camera.viewportWidth,camera.viewportHeight)))
                val scale=camera.scale.coerceIn(.01,64.0);val side=512.0/scale
                val left=floor(max(0.0,corners.minOf{it.x})/side).toInt();val top=floor(max(0.0,corners.minOf{it.y})/side).toInt()
                val right=ceil(min(geometry.width,corners.maxOf{it.x})/side).toInt();val bottom=ceil(min(geometry.height,corners.maxOf{it.y})/side).toInt()
                if(right<=left||bottom<=top)return@launchOwned
                if((right-left).toLong()*(bottom-top)>32)throw DocumentFailure(DocumentFailureKind.Limit)
                val source=(a.project as WorkbenchDocumentsProject).documentSource(d.documentId,d.revision)
                val interaction=DocumentInteraction(jobs,core);val tiles=ArrayList<DocumentDisplayTile>()
                for(y in top until bottom)for(x in left until right) {
                    ensureActive();val origin=Point(x*side,y*side)
                    val reply=interaction.tile(source,page,DocumentTileRequest(origin,scale,512u,512u),password(a,d))
                    val meta=reply.metadata as DocumentMetadata.Tile
                    tiles+=DocumentDisplayTile(meta.origin,meta.scale,meta.width,meta.height,reply.bytes)
                    if(!current(a,d,ticket,page)||viewTicket!=viewportGeneration)return@launchOwned
                }
                val binding=DocumentFrameBinding(d.sourceAssetId,d.documentId,page,d.revision,a.epoch,viewTicket)
                val frame=DocumentDisplayFrame(binding,tiles.toList());validateDocumentDisplayFrame(frame,binding)
                if(current(a,d,ticket,page)&&viewTicket==viewportGeneration)mutable.value=state.value.copy(frame=frame,message=null,passwordNeeded=false)
            } catch(cancel:CancellationException){throw cancel}
            catch(error:DocumentFailure){if(current(a,d,ticket,page)&&viewTicket==viewportGeneration) {mutable.value=state.value.copy(message=documentFailureText(error),passwordNeeded=error.kind==DocumentFailureKind.Password)}}
            catch(_:Exception){if(current(a,d,ticket,page)&&viewTicket==viewportGeneration){mutable.value=state.value.copy(message="Document tiles could not be prepared. The original is preserved.")}}
        }
    }
    public fun cancel():Unit {
        val ticket=++generation;viewportGeneration++
        val retiring=tasks.toList();retiring.forEach{it.cancel()}
        mutable.value=state.value.copy(busy=true,export=null,frame=null,message="Document work cancelled; waiting for its owned worker to retire.")
        launchOwned{withContext(NonCancellable){retiring.joinAll()};if(ticket==generation)mutable.value=state.value.copy(busy=false)}
    }
    public fun retryViewport():Unit { val previous=viewport;viewport=null;if(previous!=null)viewport(previous) }
    public fun reportFailure(error:DocumentFailure):Unit { mutable.value=state.value.copy(message=documentFailureText(error),passwordNeeded=error.kind==DocumentFailureKind.Password) }
    public suspend fun settle():Unit {
        generation++;viewportGeneration++;viewport=null
        val retiring=tasks.toList();retiring.forEach{it.cancel()}
        withContext(NonCancellable){retiring.joinAll()}
        mutable.value=state.value.copy(busy=false,frame=null,export=null)
    }
    public suspend fun close():Unit {
        generation++;viewportGeneration++;viewport=null;epoch=null;password=null
        val retiring=tasks.toList();retiring.forEach{it.cancel()}
        withContext(NonCancellable){retiring.joinAll()}
        tasks.clear();command=null;tileTask=null;noticesTask=null;noticesPending=false;mutable.value=DocumentState()
    }
}
