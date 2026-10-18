package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

public data class DocumentAttachment(public val project: WorkbenchProject, public val snapshot: DocumentSnapshot, public val epoch: Long)
public data class DocumentDisplayTile(public val origin: Point, public val scale: Double, public val width: UInt, public val height: UInt, public val rgba: ByteArray)
public data class DocumentState(public val descriptor: EditableDescriptor? = null, public val page: UInt = 0u,
    public val busy: Boolean = false, public val textTool: Boolean = false, public val regionTool: Boolean = false, public val glyphs: List<DocumentGlyph> = emptyList(),
    public val regions: List<DocumentRegion> = emptyList(), public val notices: List<String> = emptyList(),
    public val message: String? = null, public val export: DocumentExport? = null, public val passwordNeeded: Boolean = false,
    public val tiles: List<DocumentDisplayTile> = emptyList())
/** Editor attachment, epoch, source, page and full revision must all still match
 * before a background result can reach the canvas or a destination picker. */
public class DocumentController(private val scope: CoroutineScope, private val core: WorkbenchCore,
    private val attachment: () -> DocumentAttachment?, private val admitted: () -> Boolean,
    private val changePage: suspend (DocumentAttachment, UInt) -> Unit) {
    private val mutable=MutableStateFlow(DocumentState())
    public val state: StateFlow<DocumentState> = mutable
    private var task: Job?=null;private var sequence=0L;private var password: String?=null
    private var tileTask: Job?=null;private var tileTicket=0L
    private var pageTask: Job?=null
    private var viewportKey: Triple<Camera,UInt,ProjectInfo>?=null
    public fun bind(descriptor: EditableDescriptor?, page: UInt) {
        val before=mutable.value
        if(before.page!=page || before.descriptor?.revision?.stateHash!=descriptor?.revision?.stateHash) viewportKey=null
        if(before.descriptor?.sourceAssetId!=descriptor?.sourceAssetId) { task?.cancel();tileTask?.cancel();tileTicket++;sequence++;password=null;mutable.value=DocumentState(descriptor,page);if(descriptor!=null) scope.launch { val captured=sequence;while(captured==sequence && !admitted()) delay(50);if(captured==sequence) refreshNotices() } }
        else if(before.page!=page || before.descriptor?.revision?.sameDocumentRevision(descriptor?.revision ?: return)==false) {task?.cancel();tileTask?.cancel();tileTicket++;sequence++;mutable.value=before.copy(descriptor=descriptor,page=page,busy=false,glyphs=emptyList(),export=null,textTool=false,regionTool=false,message=null,tiles=emptyList())}
        else mutable.value=before.copy(descriptor=descriptor,page=page)
    }
    private fun capture(): Pair<DocumentAttachment,EditableDescriptor> {
        val attached=attachment() ?: throw DocumentFailure(DocumentFailureKind.Stale)
        val descriptor=mutable.value.descriptor ?: throw DocumentFailure(DocumentFailureKind.Unavailable)
        if(!attached.snapshot.render.revision.sameDocumentRevision(descriptor.revision) || attached.snapshot.documentId!=descriptor.documentId) throw DocumentFailure(DocumentFailureKind.Stale)
        return attached to descriptor
    }
    private fun current(attached: DocumentAttachment, descriptor: EditableDescriptor, ticket: Long, page: UInt): Boolean {
        val now=attachment() ?: return false
        return ticket==sequence && now.project===attached.project && now.epoch==attached.epoch && now.snapshot.documentId==descriptor.documentId && now.snapshot.render.revision.sameDocumentRevision(descriptor.revision) && mutable.value.page==page
    }
    private fun work(block: suspend (DocumentAttachment,EditableDescriptor,Long,UInt) -> Unit) {
        if(mutable.value.busy) return
        if(!admitted()) {mutable.value=mutable.value.copy(message="Finish the current edit or dialog before document work.");return}
        val captured=try{capture()}catch(error:DocumentFailure){mutable.value=mutable.value.copy(message=documentFailureText(error));return}
        val ticket=++sequence;val page=mutable.value.page
        mutable.value=mutable.value.copy(busy=true,message=null,export=null)
        task=scope.launch {
            try {block(captured.first,captured.second,ticket,page)}
            catch(error:CancellationException){throw error}
            catch(error:DocumentFailure){if(ticket==sequence) mutable.value=mutable.value.copy(message=documentFailureText(error),passwordNeeded=error.kind==DocumentFailureKind.Password)}
            catch(_:Exception){if(ticket==sequence) mutable.value=mutable.value.copy(message="Document work failed; the original is retained.")}
            finally {if(ticket==sequence) mutable.value=mutable.value.copy(busy=false)}
        }
    }
    public fun refreshNotices(): Unit = work { attached,descriptor,ticket,page ->
        val source=(attached.project as WorkbenchDocumentsProject).documentSource(descriptor.documentId,descriptor.revision)
        val result=installedDocumentJobs().run(DocumentRequest(if(descriptor.kind==EditableKind.Pdf)DocumentOperation.PdfInfo else DocumentOperation.SvgInfo,password ?: (attached.project as WorkbenchDocumentsProject).documentPassword(descriptor.documentId)),source.bytes)
        val info=result.metadata as? DocumentMetadata.Info ?: throw DocumentFailure(DocumentFailureKind.Verification)
        if(result.sourceBlake3!=descriptor.sourceAssetId) throw DocumentFailure(DocumentFailureKind.Verification)
        if(current(attached,descriptor,ticket,page)) mutable.value=mutable.value.copy(notices=info.notices,message=null,passwordNeeded=false)
    }
    public fun password(value: String) {if(value.length>1024 || '\u0000' in value) return;password=value;mutable.value.descriptor?.let{(attachment()?.project as? WorkbenchDocumentsProject)?.documentPassword(it.documentId,value)};viewportKey=null;page(mutable.value.page);scope.launch{pageTask?.join();refreshNotices()}}
    public fun page(index: UInt) {
        if(mutable.value.busy||!admitted())return
        val captured=try{capture()}catch(error:DocumentFailure){mutable.value=mutable.value.copy(message=documentFailureText(error));return}
        if(index>=captured.second.pages.size.toUInt())return
        mutable.value=mutable.value.copy(busy=true,export=null)
        pageTask=scope.launch{try{changePage(captured.first,index)}catch(error:CancellationException){throw error}catch(error:DocumentFailure){mutable.value=mutable.value.copy(message=documentFailureText(error))}catch(_:Exception){mutable.value=mutable.value.copy(message="The page could not be prepared; the original is retained.")}finally{mutable.value=mutable.value.copy(busy=false)}}
    }
    public fun textTool(enabled: Boolean) {if(mutable.value.descriptor?.kind!=EditableKind.Pdf || mutable.value.busy || !admitted()) return;mutable.value=mutable.value.copy(textTool=enabled,regionTool=false,glyphs=emptyList())}
    public fun regionTool(enabled: Boolean) {if(mutable.value.descriptor?.kind!=EditableKind.Pdf || mutable.value.busy || !admitted()) return;mutable.value=mutable.value.copy(textTool=false,regionTool=enabled,glyphs=emptyList())}
    public fun inputStarted() {if(mutable.value.textTool||mutable.value.regionTool){sequence++;task?.cancel();mutable.value=mutable.value.copy(textTool=false,regionTool=false,glyphs=emptyList(),busy=false)}}
    public fun select(region: Rect): Unit = work {attached,descriptor,ticket,page ->
        val source=(attached.project as WorkbenchDocumentsProject).documentSource(descriptor.documentId,descriptor.revision)
        val glyphs=DocumentInteraction(installedDocumentJobs(),core).select(source,page,region,password ?: (attached.project as WorkbenchDocumentsProject).documentPassword(descriptor.documentId))
        if(current(attached,descriptor,ticket,page)) mutable.value=mutable.value.copy(glyphs=glyphs)
    }
    public fun redactSelection() {
        val boxes=mutable.value.glyphs.mapNotNull{it.bounds};if(boxes.isEmpty() || mutable.value.regions.size>=4096) return
        val left=boxes.minOf{it.x};val top=boxes.minOf{it.y};val right=boxes.maxOf{it.x+it.width};val bottom=boxes.maxOf{it.y+it.height}
        region(Rect(left,top,right-left,bottom-top))
    }
    public fun region(rectangle: Rect) {if(!admitted() || mutable.value.busy || mutable.value.descriptor?.kind!=EditableKind.Pdf || mutable.value.regions.size>=4096 || !listOf(rectangle.x,rectangle.y,rectangle.width,rectangle.height).all(Double::isFinite) || rectangle.width<=0 || rectangle.height<=0) return;mutable.value=mutable.value.copy(regions=mutable.value.regions+DocumentRegion(mutable.value.page,rectangle),export=null)}
    public fun clearRegions() {mutable.value=mutable.value.copy(regions=emptyList(),export=null)}
    public fun export(mode: DocumentCopy, removeEncryption: Boolean = false): Unit = work {attached,descriptor,ticket,page ->
        val result=DocumentInteraction(installedDocumentJobs(),core).export(attached.project,descriptor,mode,mutable.value.regions,password ?: (attached.project as WorkbenchDocumentsProject).documentPassword(descriptor.documentId),removeEncryption)
        if(current(attached,descriptor,ticket,page)) mutable.value=mutable.value.copy(export=result,notices=result.notices,message=if(result.redacted)"Verified redaction export is ready; text and document features may be lost." else "Ordinary copy is ready; it is not a redaction export.")
    }
    public fun checkedExport(): DocumentExport? {
        val result=mutable.value.export ?: return null
        val attached=attachment() ?: return null
        if(!attached.snapshot.render.revision.sameDocumentRevision(result.revision) || mutable.value.descriptor?.sourceAssetId!=result.sourceAssetId) return null
        return result
    }
    public fun viewport(camera: Camera) {
        if(mutable.value.descriptor==null || camera.viewportWidth<=0 || camera.viewportHeight<=0 || !admitted()) return
        val captured=try{capture()}catch(_:DocumentFailure){return};val page=mutable.value.page;val ticket=++tileTicket
        val key=Triple(camera,page,captured.second.revision);if(viewportKey==key) {tileTicket--;return};viewportKey=key
        val previous=tileTask;previous?.cancel()
        tileTask=scope.launch {
            previous?.join()
            try {
                val geometry=captured.second.pages.getOrNull(page.toInt()) ?: return@launch
                val corners=core.mapPoints(camera,true,listOf(Point(0.0,0.0),Point(camera.viewportWidth,0.0),Point(0.0,camera.viewportHeight),Point(camera.viewportWidth,camera.viewportHeight)))
                val scale=camera.scale.coerceIn(0.01,64.0);val side=512.0/scale
                val left=kotlin.math.floor(corners.minOf{it.x}.coerceAtLeast(0.0)/side).toInt()
                val top=kotlin.math.floor(corners.minOf{it.y}.coerceAtLeast(0.0)/side).toInt()
                val right=kotlin.math.ceil(corners.maxOf{it.x}.coerceAtMost(geometry.width)/side).toInt()
                val bottom=kotlin.math.ceil(corners.maxOf{it.y}.coerceAtMost(geometry.height)/side).toInt()
                if(right<=left || bottom<=top || (right-left).toLong()*(bottom-top)>32) return@launch
                val source=(captured.first.project as WorkbenchDocumentsProject).documentSource(captured.second.documentId,captured.second.revision)
                val interaction=DocumentInteraction(installedDocumentJobs(),core);val tiles=ArrayList<DocumentDisplayTile>()
                for(y in top until bottom) for(x in left until right) {
                    ensureActive();val origin=Point(x*side,y*side)
                    val reply=interaction.tile(source,page,DocumentTileRequest(origin,scale,512u,512u),password ?: (captured.first.project as WorkbenchDocumentsProject).documentPassword(captured.second.documentId))
                    val meta=reply.metadata as DocumentMetadata.Tile;tiles+=DocumentDisplayTile(meta.origin,meta.scale,meta.width,meta.height,reply.bytes)
                    val now=attachment()
                    if(ticket!=tileTicket || now?.project!==captured.first.project || now.epoch!=captured.first.epoch || !now.snapshot.render.revision.sameDocumentRevision(captured.second.revision) || mutable.value.page!=page) return@launch
                    mutable.value=mutable.value.copy(tiles=tiles.toList())
                }
            } catch(error:CancellationException){throw error}
            catch(error:DocumentFailure){if(ticket==tileTicket) mutable.value=mutable.value.copy(message=documentFailureText(error))}
            catch(_:Exception){if(ticket==tileTicket) mutable.value=mutable.value.copy(message="Viewport tiles could not be prepared. The saved original is retained.")}
        }
    }
    public suspend fun close() {sequence++;tileTicket++;viewportKey=null;task?.cancelAndJoin();tileTask?.cancelAndJoin();pageTask?.cancelAndJoin();task=null;tileTask=null;pageTask=null;password=null;mutable.value=DocumentState()}
}
