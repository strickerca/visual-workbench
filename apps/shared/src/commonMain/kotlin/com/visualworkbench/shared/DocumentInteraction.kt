@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import kotlinx.coroutines.ensureActive
import kotlin.coroutines.coroutineContext
import kotlin.math.ceil
import kotlin.math.min

/** Explicit, visible limits are shared by desktop and phone. No parser runs here. */
public class DocumentInteraction(private val jobs: DocumentJobs, private val core: WorkbenchCore) {
    private fun identity(value: String): Boolean = value.length == 64 && value.all { it in "0123456789abcdef" }
    private fun dimension(value: Double?): Boolean = value != null && value.isFinite() && value > 0.0 && value <= 65_535.0
    public suspend fun inspect(source: ByteArray, kind: EditableKind, password: String? = null,
        progress: (UInt, UInt) -> Unit = { _, _ -> }): Pair<DocumentReply, List<DocumentPage>> {
        require(source.isNotEmpty() && source.size <= if(kind == EditableKind.Svg) 4 * 1024 * 1024 else 64 * 1024 * 1024)
        val first = jobs.run(DocumentRequest(if(kind == EditableKind.Pdf) DocumentOperation.PdfInfo else DocumentOperation.SvgInfo, password), source)
        coroutineContext.ensureActive()
        val info = first.metadata as? DocumentMetadata.Info ?: throw DocumentFailure(DocumentFailureKind.Verification)
        if(info.kind != kind || info.pages !in 1u..1000u) throw DocumentFailure(DocumentFailureKind.Verification)
        if (!identity(first.sourceBlake3) || (kind == EditableKind.Svg &&
            (info.pages != 1u || !dimension(info.width) || !dimension(info.height))))
            throw DocumentFailure(DocumentFailureKind.Verification)
        val pages = ArrayList<DocumentPage>()
        for(index in 0u until info.pages) {
            coroutineContext.ensureActive()
            val now =  SystemTime.nowMs()
            val layer = core.newId(now.toULong())
            val page = if(kind == EditableKind.Pdf) {
                val reply = jobs.run(DocumentRequest(DocumentOperation.Page(index), password), source)
                if(reply.sourceBlake3 != first.sourceBlake3) throw DocumentFailure(DocumentFailureKind.Verification)
                val result = reply.metadata as? DocumentMetadata.Page ?: throw DocumentFailure(DocumentFailureKind.Verification)
                if(result.index != index || !dimension(result.width) || !dimension(result.height) || result.crop.size != 4 || result.crop.any { !it.isFinite() } || result.quarterTurns > 3u) throw DocumentFailure(DocumentFailureKind.Verification)
                DocumentPage(index,result.width,result.height,result.crop,result.quarterTurns,layer)
            } else {
                val width = info.width ?: throw DocumentFailure(DocumentFailureKind.Verification)
                val height = info.height ?: throw DocumentFailure(DocumentFailureKind.Verification)
                DocumentPage(0u,width,height,listOf(0.0,0.0,width,height),0u,layer)
            }
            pages += page; progress(index + 1u,info.pages)
        }
        coroutineContext.ensureActive()
        return first to pages
    }
    public suspend fun background(source: EditableSource, page: UInt, password: String? = null): BackgroundImage {
        val geometry = source.descriptor.pages.getOrNull(page.toInt()) ?: throw DocumentFailure(DocumentFailureKind.Invalid)
        val width = ceil(geometry.width).toInt(); val height = ceil(geometry.height).toInt()
        // This first display is one sample per D unit. Zoom tiles use tile().
        val count = width.toLong() * height
        if(width <= 0 || height <= 0 || count > 16_000_000 || ceil(width / 512.0) * ceil(height / 512.0) > 1024) throw DocumentFailure(DocumentFailureKind.Limit)
        val rgba = ByteArray((count * 4).toInt())
        for(y in 0 until height step 512) for(x in 0 until width step 512) {
            coroutineContext.ensureActive()
            val tile = tile(source,page,DocumentTileRequest(Point(x.toDouble(),y.toDouble()),1.0,min(512,width-x).toUInt(),min(512,height-y).toUInt()),password)
            val meta = tile.metadata as DocumentMetadata.Tile
            for(row in 0 until meta.height.toInt()) tile.bytes.copyInto(rgba,((y+row)*width+x)*4,row*meta.width.toInt()*4,(row+1)*meta.width.toInt()*4)
        }
        return BackgroundImage(width.toUInt(),height.toUInt(),rgba,source.descriptor.sourceAssetId,8u,byteArrayOf())
    }
    public suspend fun tile(source: EditableSource, page: UInt, tile: DocumentTileRequest, password: String? = null): DocumentReply {
        if(source.descriptor.pages.none { it.index == page }) throw DocumentFailure(DocumentFailureKind.Invalid)
        val reply = jobs.run(DocumentRequest(DocumentOperation.Tile(source.descriptor.kind,page,tile),password),source.bytes)
        val meta = reply.metadata as? DocumentMetadata.Tile ?: throw DocumentFailure(DocumentFailureKind.Verification)
        if(reply.sourceBlake3 != source.descriptor.sourceAssetId ||
            meta.page != (if(source.descriptor.kind == EditableKind.Pdf) page else null) || meta.origin != tile.origin ||
            meta.scale != tile.scale.toFloat().toDouble() || meta.width != tile.width || meta.height != tile.height)
            throw DocumentFailure(DocumentFailureKind.Verification)
        val binding = DocumentFrameBinding(source.descriptor.sourceAssetId,source.descriptor.documentId,page,source.descriptor.revision,0,0)
        validateDocumentDisplayFrame(DocumentDisplayFrame(binding,listOf(DocumentDisplayTile(meta.origin,meta.scale,meta.width,meta.height,reply.bytes))),binding)
        return reply
    }
    public suspend fun select(source: EditableSource, page: UInt, region: Rect, password: String? = null): List<DocumentGlyph> {
        if(source.descriptor.kind != EditableKind.Pdf) throw DocumentFailure(DocumentFailureKind.Unsupported)
        val reply = jobs.run(DocumentRequest(DocumentOperation.Selection(page,region),password),source.bytes)
        if(reply.sourceBlake3 != source.descriptor.sourceAssetId) throw DocumentFailure(DocumentFailureKind.Verification)
        val text = reply.metadata as? DocumentMetadata.Text ?: throw DocumentFailure(DocumentFailureKind.Verification)
        if(text.page != page) throw DocumentFailure(DocumentFailureKind.Verification)
        return text.glyphs
    }
    public suspend fun export(project: WorkbenchProject, descriptor: EditableDescriptor, mode: DocumentCopy,
        regions: List<DocumentRegion> = emptyList(), password: String? = null, explicitlyRemoveEncryption: Boolean = false): DocumentExport {
        val native = project as? WorkbenchDocumentsProject ?: throw DocumentFailure(DocumentFailureKind.Unavailable)
        if(!project.info().sameDocumentRevision(descriptor.revision)) throw DocumentFailure(DocumentFailureKind.Stale)
        if(mode == DocumentCopy.PcRedaction && !jobs.pcRedaction) throw DocumentFailure(DocumentFailureKind.Unsupported)
        if((descriptor.kind == EditableKind.Svg) != (mode == DocumentCopy.SvgAnnotations)) throw DocumentFailure(DocumentFailureKind.Invalid)
        val source = native.documentSource(descriptor.documentId,descriptor.revision)
        if(source.descriptor != descriptor) throw DocumentFailure(DocumentFailureKind.Verification)
        val overlays = if(mode in setOf(DocumentCopy.Annotated,DocumentCopy.Flattened,DocumentCopy.SvgAnnotations)) {
            val result = ArrayList<DocumentOverlay>()
            for(page in descriptor.pages) {
                val snapshot = native.documentPage(descriptor.documentId,page.index)
                if(!snapshot.render.revision.sameDocumentRevision(descriptor.revision)) throw DocumentFailure(DocumentFailureKind.Stale)
                result += native.documentOverlays(descriptor.documentId,page.index,descriptor.revision)
            }
            result
        } else emptyList()
        coroutineContext.ensureActive()
        val reply = jobs.run(DocumentRequest(DocumentOperation.Copy(mode,overlays,regions,explicitlyRemoveEncryption=explicitlyRemoveEncryption),password),source.bytes)
        val copy = reply.metadata as? DocumentMetadata.Copy ?: throw DocumentFailure(DocumentFailureKind.Verification)
        if(reply.sourceBlake3 != descriptor.sourceAssetId || copy.mode != mode || !project.info().sameDocumentRevision(descriptor.revision)) throw DocumentFailure(DocumentFailureKind.Stale)
        val expectedRedaction = mode in setOf(DocumentCopy.PcRedaction,DocumentCopy.EveryPageRasterRedaction)
        if (copy.bytes != reply.bytes.size.toULong() || copy.redacted != expectedRedaction || !identity(reply.outputBlake3))
            throw DocumentFailure(DocumentFailureKind.Verification)
        coroutineContext.ensureActive()
        return DocumentExport(reply.bytes,reply.outputBlake3,mode,copy.redacted,descriptor.sourceAssetId,descriptor.revision,copy.notices)
    }
}
internal expect object SystemTime { public fun nowMs(): Long }

public fun documentNoticeText(notice: String): String = when(notice) {
    "system_fonts_disabled" -> "System fonts are disabled; embedded and bundled fonts may look different."
    "non_interactive_forms" -> "Forms are shown without interactive fields."
    "original_document_features_retained" -> "Ordinary copies retain original document features and are not redaction exports."
    "resource_denied" -> "External images, files and network resources were refused."
    "script_or_event_ignored" -> "Scripts and event handlers were ignored."
    "foreign_object_ignored" -> "foreignObject content was omitted."
    "animation_ignored" -> "Animation was omitted."
    "missing_font", "missing_glyph" -> "Some fonts or characters are unavailable."
    "unsupported_filter", "unsupported_element", "stylesheet_limited" -> "Some SVG styling or elements are not supported."
    else -> "The document renderer reported a fidelity limit."
}
public fun documentFailureText(error: DocumentFailure): String = when(error.kind) {
    DocumentFailureKind.Unavailable -> "PDF/SVG support is unavailable in this installation. The original is preserved."
    DocumentFailureKind.Password -> "This PDF requires a correct password."
    DocumentFailureKind.Unsupported -> "This content cannot be exported by that method. Choose a different method explicitly; the original is retained."
    DocumentFailureKind.Limit -> "This document exceeds the bounded page, tile, memory or export limit. The original is retained."
    DocumentFailureKind.Stale -> "The project, page or revision changed. Retry from the current document."
    DocumentFailureKind.Isolation -> "The document worker could not retire or isolate safely. Further document jobs are blocked."
    DocumentFailureKind.Timeout -> "Document processing exceeded its two-minute limit."
    DocumentFailureKind.Cancelled -> "Document work was cancelled before result adoption."
    else -> "Document work failed its validation. The original is retained."
}
