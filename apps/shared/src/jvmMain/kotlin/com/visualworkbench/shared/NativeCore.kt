@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import com.visualworkbench.bindings.core.*
import com.visualworkbench.bindings.core.Camera as NCamera
import com.visualworkbench.bindings.core.Point as NPoint
import com.visualworkbench.bindings.core.Transform as NTransform
import com.visualworkbench.bindings.core.ProjectInfo as NInfo
import com.visualworkbench.bindings.core.StrokeOptions as NStrokeOptions
import com.visualworkbench.bindings.core.SampleBatch as NBatch
import com.visualworkbench.bindings.core.Contours as NContours
import com.visualworkbench.bindings.core.InkUpdate as NInk
import com.visualworkbench.bindings.core.ObjectStyle as NStyle
import com.visualworkbench.bindings.core.Outline as NOutline
import com.visualworkbench.bindings.core.RenderList as NRender
import com.visualworkbench.bindings.core.EditOptions as NEditOptions
import com.visualworkbench.bindings.core.EditCommand as NEdit
import com.visualworkbench.bindings.core.ImageFormat as NFormat
import com.visualworkbench.bindings.core.ExportOptions as NExportOptions
import com.visualworkbench.bindings.core.ChangeKind as NChange
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.withContext
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

public actual fun workbenchCore(): WorkbenchCore { prepareCoreLibrary(); check(bindingContractVersion() == 1u); return NativeCore() }

internal fun mapFailure(error: CoreException): CoreFailure = CoreFailure(when(error) {
    is CoreException.Invalid -> CoreFailureKind.Invalid
    is CoreException.Backpressure -> CoreFailureKind.Backpressure
    is CoreException.Closed -> CoreFailureKind.Closed
    is CoreException.Cancelled -> CoreFailureKind.Cancelled
    is CoreException.Storage -> CoreFailureKind.Storage
    is CoreException.Unsupported -> CoreFailureKind.Unsupported
    is CoreException.Worker -> CoreFailureKind.Worker
    is CoreException.Stroke -> CoreFailureKind.Stroke
    is CoreException.Raster -> CoreFailureKind.Raster
    is CoreException.ImageCodecUnavailable -> CoreFailureKind.ImageCodecUnavailable
},error)
private suspend fun <T> native(block: suspend (Cancellation) -> T): T = withContext(Dispatchers.Default) {
    val cancellation = Cancellation()
    try { block(cancellation) } catch (error: CancellationException) { cancellation.cancel(); throw error }
    catch (error: CoreException) { throw mapFailure(error) } finally { cancellation.destroy() }
}
private class NativeCore : WorkbenchCore, WorkbenchStreamingCore, WorkbenchDocumentsCore, WorkbenchGiantImportCore {
    override suspend fun openGiantOriginal(options: GiantOriginalOptions): GiantPickedOriginal = openGiantOriginalNative(options)
    override suspend fun createDocument(options: CreateDocument): WorkbenchProject = createDocumentNative(options) {
        NativeProject(it).also { project -> project.documentPassword(options.documentId,options.password) }
    }
    override suspend fun createFile(options: CreateFileProject): WorkbenchProject = createFileNative(options) { NativeProject(it) }
    override fun newId(unixMs: ULong): String = generateId(unixMs)
    override fun newDeviceId(): String = generateDeviceId()
    override suspend fun create(options: CreateProject): WorkbenchProject = acquireNative({createImageProject(CreateImageProject(options.path,options.projectId,options.documentId,options.layerId,options.deviceId,options.title,options.source,options.nowMs),it)},{NativeProject(it)},{try{it.closeSession()}finally{it.destroy()}})
    override suspend fun open(path: String): WorkbenchProject = acquireNative({openProject(path,it)},{NativeProject(it)},{try{it.closeSession()}finally{it.destroy()}})
    override fun cameraMatrix(camera: Camera,inverse: Boolean): Transform = com.visualworkbench.bindings.core.cameraMatrix(camera.native(),inverse).common()
    override fun mapPoints(camera: Camera,inverse: Boolean,points: List<Point>): List<Point> = cameraMapPoints(camera.native(),inverse,points.map { it.native() }).map { it.common() }
    override suspend fun layoutText(text: String,font: String,size: Float): TextLayout = native { cancel ->
        val value = com.visualworkbench.bindings.core.layoutText(text,font,size,cancel)
        TextLayout(value.algorithmVersion,value.glyphs.map { GlyphOutline(it.glyphId,it.cluster,it.font,it.x,it.y,it.advance,it.outline.map { p -> p.common() }) },value.width,value.height,value.lineHeight)
    }
}
private class NativeProject(val handle: ProjectSession, private val giantOwner: GiantImportOwner<ProjectSession>? = null) : WorkbenchProject, WorkbenchStreamingProject, WorkbenchFileTransfers, WorkbenchOriginalFormats, WorkbenchDocumentsProject, WorkbenchGiantImagesProject {
    suspend fun <T> withGiantProject(block: suspend (ProjectSession) -> T): T = closeMutex.withLock {
        if (closed.get()) throw GiantImageFailure(GiantImageFailureKind.Closed)
        if (giantOwner != null) giantOwner.call(block) else block(handle)
    }
    override suspend fun openGiantImage(binding:WorkflowBinding,diskCacheBytes:ULong,ramCacheBytes:ULong):GiantImage {
        if(closed.get())throw GiantImageFailure(GiantImageFailureKind.Closed)
        val image=openGiantImageNative(handle,binding,diskCacheBytes,ramCacheBytes)
        try {
            currentCoroutineContext().ensureActive()
            if(closed.get())throw GiantImageFailure(GiantImageFailureKind.Closed)
            return image
        } catch(error:Throwable) {
            try{disposeUnadoptedGiantImage(image)}catch(cleanup:Throwable){error.addSuppressed(cleanup)}
            throw error
        }
    }
    private val documentPasswords = mutableMapOf<String,String>()
    private val documentPages = mutableMapOf<String,UInt>()
    private fun checkDocumentOpen() { if(closed.get()) throw CoreFailure(CoreFailureKind.Closed) }
    override fun documentPassword(documentId: String): String? { checkDocumentOpen(); return documentPasswords[documentId] }
    override fun documentPassword(documentId: String,password: String?) {
        checkDocumentOpen()
        require(password == null || (password.length <= 1024 && '\u0000' !in password))
        if(password == null) documentPasswords.remove(documentId) else documentPasswords[documentId] = password
    }
    override fun selectedDocumentPage(documentId: String): UInt { checkDocumentOpen(); return documentPages[documentId] ?: 0u }
    override suspend fun documentDescriptor(documentId: String): EditableDescriptor? = documentNativeCall {
        checkDocumentOpen(); handle.editableDocumentDescriptor(documentId,it)?.documentCommon()
    }
    override suspend fun documentSource(documentId: String,expected: ProjectInfo): EditableSource = documentNativeCall {
        checkDocumentOpen(); val value=handle.editableDocumentSource(documentId,expected.documentNative(),it)
        EditableSource(value.descriptor.documentCommon(),value.bytes)
    }
    override suspend fun documentPage(documentId: String,page: UInt): DocumentSnapshot = documentNativeCall {
        checkDocumentOpen(); handle.editableDocumentPage(documentId,page,it).documentCommon().copy(page=page)
    }
    override suspend fun selectDocumentPage(documentId: String,page: UInt): DocumentSnapshot {
        val value=documentPage(documentId,page)
        currentCoroutineContext().ensureActive();checkDocumentOpen();documentPages[documentId]=page
        return value
    }
    override suspend fun documentOverlays(documentId: String,page: UInt,expected: ProjectInfo): List<DocumentOverlay> = documentNativeCall {
        checkDocumentOpen(); handle.editableDocumentOverlays(documentId,page,expected.documentNative(),it).map { value ->
            DocumentOverlay(value.page,value.contours.map { contour -> contour.map { point -> Point(point.x,point.y) } },Transform(value.transform.a,value.transform.b,value.transform.c,value.transform.d,value.transform.e,value.transform.f),value.rgba)
        }
    }
    override suspend fun publishDocumentCopy(documentId: String,destination: String,copy: DocumentExport): String = documentNativeCall {
        checkDocumentOpen(); handle.publishEditableCopy(PublishEditableCopy(documentId,copy.sourceAssetId,copy.revision.documentNative(),destination,copy.bytes,copy.blake3),it)
    }
    override suspend fun originalFormatInfo(binding: WorkflowBinding): OriginalFormatInfo {
        if (closed.get()) throw WorkflowFailure(WorkflowFailureKind.Closed)
        return originalFormatInfoNative(handle, binding)
    }
    override suspend fun preflightFile(options: FileExportOptions): FilePreflight {
        if (closed.get()) throw TransferFailure(TransferFailureKind.Closed)
        return preflightFileNative(handle, options)
    }
    override suspend fun exportTransfer(options: FileExportOptions): FileExportResult {
        if (closed.get()) throw TransferFailure(TransferFailureKind.Closed)
        return exportTransferNative(handle, options)
    }
    override suspend fun attachFile(options: AttachFileOptions): AttachedAsset {
        if (closed.get()) throw TransferFailure(TransferFailureKind.Closed)
        return attachFileNative(handle, options)
    }
    override suspend fun exportFile(options: FileExportOptions): FileExportResult { if (closed.get()) throw CoreFailure(CoreFailureKind.Closed); return exportFileNative(handle, options) }
    private val closed = java.util.concurrent.atomic.AtomicBoolean(false)
    private val closeMutex = Mutex()
    private var retirementFailure: Throwable? = null
    private suspend fun <T> call(block: suspend (Cancellation) -> T): T = native { if(closed.get()) throw CoreFailure(CoreFailureKind.Closed); block(it) }
    override val changes: Flow<ProjectChange> = callbackFlow {
        val subscription = handle.subscribe(object : StateListener {
            override fun onChange(change: StateChange) { trySend(ProjectChange(change.sequence,when(change.kind) {
                NChange.OPENED -> ChangeKind.Opened; NChange.COMMITTED -> ChangeKind.Committed; NChange.EXPORT_STARTED -> ChangeKind.ExportStarted; NChange.EXPORT_READY -> ChangeKind.ExportReady; NChange.EXPORT_FAILED -> ChangeKind.ExportFailed; NChange.CLOSED -> ChangeKind.Closed
            },change.project.common())); if(change.kind==NChange.CLOSED) close() }
        })
        awaitClose { subscription.unsubscribe(); subscription.destroy() }
    }.buffer(Channel.CONFLATED)
    override suspend fun info(): ProjectInfo = call { handle.info().common() }
    override suspend fun document(documentId: String): DocumentSnapshot {
        if(documentDescriptor(documentId)!=null)return documentPage(documentId,selectedDocumentPage(documentId))
        return imageDocument(documentId)
    }
    private suspend fun imageDocument(documentId: String): DocumentSnapshot = call { cancel ->
        val d=handle.documentSnapshot(documentId,cancel)
        DocumentSnapshot(d.documentId,d.title,d.width,d.height,d.bitDepth,d.layers.map { LayerInfo(it.id,it.name,it.visible,it.locked,it.opacity,it.blend) },d.render.common(),hasToneAdjustments=d.hasToneAdjustments)
    }
    override suspend fun background(documentId: String,memoryBudgetBytes: ULong,assumeUntaggedSrgb: Boolean): BackgroundImage = call { cancel ->
        val b=handle.backgroundImage(documentId,memoryBudgetBytes,assumeUntaggedSrgb,cancel)
        BackgroundImage(b.width,b.height,b.rgba,b.sourceAssetId,b.sourceBitDepth,b.iccProfile)
    }
    override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke {
        if(closed.get())throw CoreFailure(CoreFailureKind.Closed)
        return acquireNative({handle.beginStroke(NStrokeOptions(options.gestureId,options.transactionId,options.objectId,options.documentId,options.layerId,options.deviceId,options.lamport,options.createdAtMs,options.family,options.width,options.rgba,options.stabilization,options.pressureCurve))},{NativeStroke(it)},{try{it.shutdown()}finally{it.destroy()}})
    }
    override suspend fun edit(options: EditOptions,commands: List<EditCommand>): ProjectInfo = call {
        if ((options.expectedHostSeq == null) != (options.expectedStateHash == null)) throw CoreFailure(CoreFailureKind.Invalid)
        val descriptor = documentDescriptor(options.documentId)
        val page = descriptor?.let { selectedDocumentPage(options.documentId) }
        if (options.page != null && options.page != page) throw CoreFailure(CoreFailureKind.Invalid)
        val visible = if (page != null && options.expectedHostSeq == null) info() else null
        val expected = options.expectedHostSeq?.let { seq -> EditPrecondition(seq, requireNotNull(options.expectedStateHash), page) }
            ?: visible?.let { value -> EditPrecondition(value.hostSeq,value.stateHash,page) }
        handle.applyEditAt(options.native(),commands.map { it.native() },expected,options.gestureId,it).common()
    }
    override suspend fun undoRedo(options: EditOptions,redo: Boolean): ProjectInfo = call { handle.undoRedo(options.native(),redo,it).common() }
    override suspend fun render(documentId: String,rectangle: Rect?): RenderList {
        if(documentDescriptor(documentId)!=null)return documentPage(documentId,selectedDocumentPage(documentId)).render
        return call { handle.renderList(documentId,rectangle?.native(),it).common() }
    }
    override suspend fun export(options: ExportOptions): ExportResult = call { cancel ->
        val e=handle.exportImage(NExportOptions(options.documentId,options.format.native(),options.marked,options.region?.native(),options.matteRgb,options.convertToSrgb,options.assumeUntaggedSrgb,options.allowDepthReduction,options.memoryBudgetBytes),cancel)
        ExportResult(e.bytes,e.blake3,e.metadataJson,e.revision.common())
    }
    override suspend fun close(): Unit = releaseNative {
        closeMutex.withLock {
            if (giantOwner != null) {
                // Refuse new calls on an attempted close, but always retry the
                // exact owner; a failed close must not become a no-op success.
                closed.set(true)
                giantOwner.close()
                documentPasswords.clear(); documentPages.clear()
                return@withLock
            }
            // Ordinary worker shutdown errors are reported after its state is
            // destroyed. Preserve that result; do not retry native destruction
            // or turn a later close into a no-op success.
            retirementFailure?.let { throw it }
            if (closed.get()) return@withLock
            // A cancelled open may retain a native child whose close failed.
            // Its actual retirement must settle before the parent worker closes.
            drainUnadoptedGiants()
            var failure: Throwable? = null
            try { handle.closeSession() }
            catch(error:CoreException) { failure = mapFailure(error) }
            catch(error:Throwable) { failure = error }
            closed.set(true);documentPasswords.clear();documentPages.clear()
            try { handle.destroy() }
            catch(error:Throwable) {
                val prior = failure
                if(prior == null) failure = error else if(prior !== error) prior.addSuppressed(error)
            }
            failure?.let { retirementFailure = it; throw it }
        }
    }
}
private class NativeStroke(val handle: StrokeGesture) : WorkbenchStroke {
    private val closed = java.util.concurrent.atomic.AtomicBoolean(false)
    private val closeMutex = Mutex()
    private suspend fun <T> call(block: suspend (Cancellation) -> T): T = native {
        if(closed.get()) throw CoreFailure(CoreFailureKind.Closed)
        block(it)
    }
    override suspend fun append(batch: SampleBatch): InkUpdate = call { handle.appendSamples(batch.native()).common() }
    override suspend fun predict(batch: SampleBatch): InkUpdate = call { handle.previewSamples(batch.native()).common() }
    override suspend fun commit(): ProjectInfo = call { handle.commit(it).common() }
    override fun cancel() { if(!closed.get()) try { handle.cancel() } catch(error:CoreException) { throw mapFailure(error) } }
    override suspend fun dispose(): Unit = releaseNative {
        closeMutex.withLock { if(!closed.getAndSet(true)) {
            try { handle.shutdown() } catch(error:CoreException) { throw mapFailure(error) }
            finally { handle.destroy() }
        } }
    }
}
private fun Camera.native(): NCamera = NCamera(center.native(),scale,rotation,viewportWidth,viewportHeight)
private fun Point.native(): NPoint = NPoint(x,y)
private fun NPoint.common(): Point = Point(x,y)
private fun Rect.native(): QueryRect = QueryRect(x,y,width,height)
private fun QueryRect.common(): Rect = Rect(x,y,width,height)
private fun Transform.native(): NTransform = NTransform(a,b,c,d,e,f)
private fun NTransform.common(): Transform = Transform(a,b,c,d,e,f)
private fun NInfo.common(): ProjectInfo = ProjectInfo(projectId,title,deviceId,nextLamport,canUndo,canRedo,hostSeq,stateHash,documentIds)
private fun NContours.common(): Contours = Contours(x.toLongArray(),y.toLongArray(),ends.toUIntArray())
private fun NInk.common(): InkUpdate = InkUpdate(sequence,firstPolygon,sampleCount,contours.common())
private fun SampleBatch.native(): NBatch = NBatch(sequence,x.toList(),y.toList(),timeMs.toList(),pressure.toList(),tilt.toList(),orientation.toList())
private fun ObjectStyle.native(): NStyle = NStyle(rgba,width,screenConstantWidth,fill)
private fun NStyle.common(): ObjectStyle = ObjectStyle(rgba,width,screenConstantWidth,fill)
private fun NOutline.common(): Outline = when(this) {
    is NOutline.Move -> Outline.Move(x,y); is NOutline.Line -> Outline.Line(x,y); is NOutline.Quad -> Outline.Quad(x1,y1,x,y); is NOutline.Cubic -> Outline.Cubic(x1,y1,x2,y2,x,y); is NOutline.Close -> Outline.Close
}
private fun DrawShape.common(): Shape = when(this) {
    is DrawShape.Stroke -> Shape.Stroke(family); is DrawShape.Line -> Shape.Line(points.map { it.common() }); is DrawShape.Arrow -> Shape.Arrow(points.map { it.common() }); is DrawShape.Rectangle -> Shape.Rectangle(rectangle.common()); is DrawShape.Ellipse -> Shape.Ellipse(rectangle.common()); is DrawShape.Polygon -> Shape.Polygon(points.map { it.common() },closed)
    is DrawShape.Text -> Shape.Text(anchor.common(),text,font,size,outline.map { it.common() }); is DrawShape.Marker -> Shape.Marker(number,point.common(),rectangle?.common()); is DrawShape.Guide -> Shape.Guide(rectangle.common()); is DrawShape.Result -> Shape.Result(assetId,resultId); is DrawShape.Adjustment -> Shape.Adjustment
}
private fun NRender.common(): RenderList = RenderList(revision.common(),items.map { RenderItem(it.objectId,it.layerId,it.layerOpacity,it.layerBlend,it.bounds.common(),it.strokeContours.common(),it.transform.common(),it.style.common(),it.shape.common(),it.locked) })
internal fun nativeRenderItem(item: com.visualworkbench.bindings.core.RenderItem): RenderItem = RenderItem(item.objectId,item.layerId,item.layerOpacity,item.layerBlend,item.bounds.common(),item.strokeContours.common(),item.transform.common(),item.style.common(),item.shape.common(),item.locked)
internal fun nativeProjectHandle(project: WorkbenchProject): ProjectSession = (project as? NativeProject)?.handle ?: throw SessionFailure(SessionFailureKind.Invalid)
private fun EditOptions.native(): NEditOptions = NEditOptions(transactionId,documentId,deviceId,lamport,createdAtMs)
private fun Shape.newNative(): NewShape = when(this) {
    is Shape.Line -> NewShape.Line(points.map { it.native() }); is Shape.Arrow -> NewShape.Arrow(points.map { it.native() }); is Shape.Rectangle -> NewShape.Rectangle(rectangle.native()); is Shape.Ellipse -> NewShape.Ellipse(rectangle.native()); is Shape.Text -> NewShape.Text(anchor.native(),text,font,size)
    else -> throw CoreFailure(CoreFailureKind.Unsupported)
}
private fun EditCommand.native(): NEdit = when(this) {
    is EditCommand.Create -> NEdit.Create(objectId,layerId,shape.newNative(),style.native(),transform.native()); is EditCommand.Delete -> NEdit.Delete(objectId); is EditCommand.SetTransform -> NEdit.Transform(objectId,transform.native())
    is EditCommand.SetText -> NEdit.SetText(objectId,text,font,size)
    is EditCommand.SetStyle -> NEdit.SetStyle(objectId,style.native())
}
private fun ImageFormat.native(): NFormat = when(this) { ImageFormat.Png8 -> NFormat.Png8; ImageFormat.Png16 -> NFormat.Png16; ImageFormat.Bmp8 -> NFormat.Bmp8; ImageFormat.Tiff8 -> NFormat.Tiff8; ImageFormat.Tiff16 -> NFormat.Tiff16; is ImageFormat.Jpeg -> NFormat.Jpeg(quality); ImageFormat.WebpLossless -> NFormat.WebpLossless; is ImageFormat.WebpLossy -> NFormat.WebpLossy(quality) }

internal fun nativeStrokeHandle(stroke: WorkbenchStroke): StrokeGesture = (stroke as? NativeStroke)?.handle ?: throw SessionFailure(SessionFailureKind.Invalid)

/** Ownership transfers only at the caller's cancellation-safe handoff. */
internal fun wrapNativeProject(handle: ProjectSession): WorkbenchProject = NativeProject(handle)
internal fun wrapNativeGiantProject(owner: GiantImportOwner<ProjectSession>): WorkbenchProject = NativeProject(owner.value, owner)
internal suspend fun <T> withNativeGiantProject(project: WorkbenchProject, block: suspend (ProjectSession) -> T): T {
    val native = project as? NativeProject ?: throw GiantImageFailure(GiantImageFailureKind.Invalid)
    return native.withGiantProject(block)
}
