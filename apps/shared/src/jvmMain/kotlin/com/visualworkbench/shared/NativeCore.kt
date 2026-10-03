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
},error)
private suspend fun <T> native(block: suspend (Cancellation) -> T): T = withContext(Dispatchers.Default) {
    val cancellation = Cancellation()
    try { block(cancellation) } catch (error: CancellationException) { cancellation.cancel(); throw error }
    catch (error: CoreException) { throw mapFailure(error) } finally { cancellation.destroy() }
}
private class NativeCore : WorkbenchCore, WorkbenchStreamingCore {
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
private class NativeProject(val handle: ProjectSession) : WorkbenchProject, WorkbenchStreamingProject, WorkbenchFileTransfers {
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
    override suspend fun document(documentId: String): DocumentSnapshot = call { cancel ->
        val d=handle.documentSnapshot(documentId,cancel)
        DocumentSnapshot(d.documentId,d.title,d.width,d.height,d.bitDepth,d.layers.map { LayerInfo(it.id,it.name,it.visible,it.locked,it.opacity,it.blend) },d.render.common())
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
        val expected = options.expectedHostSeq?.let { seq -> EditPrecondition(seq, requireNotNull(options.expectedStateHash)) }
        handle.applyEditAt(options.native(),commands.map { it.native() },expected,options.gestureId,it).common()
    }
    override suspend fun undoRedo(options: EditOptions,redo: Boolean): ProjectInfo = call { handle.undoRedo(options.native(),redo,it).common() }
    override suspend fun render(documentId: String,rectangle: Rect?): RenderList = call { handle.renderList(documentId,rectangle?.native(),it).common() }
    override suspend fun export(options: ExportOptions): ExportResult = call { cancel ->
        val e=handle.exportImage(NExportOptions(options.documentId,options.format.native(),options.marked,options.region?.native(),options.matteRgb,options.convertToSrgb,options.assumeUntaggedSrgb,options.allowDepthReduction,options.memoryBudgetBytes),cancel)
        ExportResult(e.bytes,e.blake3,e.metadataJson,e.revision.common())
    }
    override suspend fun close(): Unit = releaseNative {
        closeMutex.withLock { if(!closed.get()) {
            try { handle.closeSession() } catch(error:CoreException) { throw mapFailure(error) }
            finally { closed.set(true);handle.destroy() }
        } }
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
    is DrawShape.Text -> Shape.Text(anchor.common(),text,font,size,outline.map { it.common() }); is DrawShape.Marker -> Shape.Marker(number,point.common(),rectangle?.common()); is DrawShape.Guide -> Shape.Guide(rectangle.common()); is DrawShape.Result -> Shape.Result(assetId); is DrawShape.Adjustment -> Shape.Adjustment
}
private fun NRender.common(): RenderList = RenderList(revision.common(),items.map { RenderItem(it.objectId,it.layerId,it.layerOpacity,it.layerBlend,it.bounds.common(),it.strokeContours.common(),it.transform.common(),it.style.common(),it.shape.common(),it.locked) })
internal fun nativeRenderItem(item: com.visualworkbench.bindings.core.RenderItem): RenderItem = RenderItem(item.objectId,item.layerId,item.layerOpacity,item.layerBlend,item.bounds.common(),item.strokeContours.common(),item.transform.common(),item.style.common(),item.shape.common(),item.locked)
internal fun nativeProjectHandle(project: WorkbenchProject): ProjectSession = (project as? NativeProject)?.handle ?: throw SessionFailure(SessionFailureKind.Invalid)
internal fun wrapNativeProject(project: ProjectSession): WorkbenchProject = NativeProject(project)
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
private fun ImageFormat.native(): NFormat = when(this) { ImageFormat.Png8 -> NFormat.Png8; ImageFormat.Png16 -> NFormat.Png16; is ImageFormat.Jpeg -> NFormat.Jpeg(quality); ImageFormat.WebpLossless -> NFormat.WebpLossless; is ImageFormat.WebpLossy -> NFormat.WebpLossy(quality) }

internal fun nativeStrokeHandle(stroke: WorkbenchStroke): StrokeGesture = (stroke as? NativeStroke)?.handle ?: throw SessionFailure(SessionFailureKind.Invalid)
