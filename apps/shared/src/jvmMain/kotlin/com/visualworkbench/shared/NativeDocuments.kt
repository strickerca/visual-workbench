@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import com.visualworkbench.bindings.core.*
import com.visualworkbench.bindings.core.ProjectInfo as NativeInfo
import com.visualworkbench.bindings.core.DocumentSnapshot as NativeSnapshot
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

internal actual object SystemTime { public actual fun nowMs(): Long = System.currentTimeMillis() }
internal fun NativeInfo.documentCommon(): ProjectInfo = ProjectInfo(projectId,title,deviceId,nextLamport,canUndo,canRedo,hostSeq,stateHash,documentIds)
internal fun NativeSnapshot.documentCommon(): DocumentSnapshot = DocumentSnapshot(documentId,title,width,height,bitDepth,layers.map{LayerInfo(it.id,it.name,it.visible,it.locked,it.opacity,it.blend)},RenderList(render.revision.documentCommon(),render.items.map(::nativeRenderItem)))
internal fun EditableDocumentDescriptor.documentCommon(): EditableDescriptor = EditableDescriptor(documentId,sourceAssetId,if(kind == EditableDocumentKind.PDF) EditableKind.Pdf else EditableKind.Svg,pages.map{DocumentPage(it.page,it.width,it.height,it.crop,it.quarterTurns,it.layerId)},revision.documentCommon())
internal fun CreateDocument.documentNative(): CreateEditableDocument = CreateEditableDocument(path,projectId,documentId,deviceId,title,source,sourceBlake3,if(kind == EditableKind.Pdf) EditableDocumentKind.PDF else EditableDocumentKind.SVG,pages.map{DocumentPageGeometry(it.index,it.width,it.height,it.crop,it.quarterTurns,it.layerId)},nowMs)
internal fun ProjectInfo.documentNative(): NativeInfo = NativeInfo(projectId,title,deviceId,nextLamport,canUndo,canRedo,hostSeq,stateHash,documentIds)
internal suspend fun <T> documentNativeCall(block: suspend (Cancellation) -> T): T = settledStream(block = block)
internal fun documentJobFailure(error: DocumentJobException): DocumentFailure = DocumentFailure(when(error) {
    is DocumentJobException.Unavailable -> DocumentFailureKind.Unavailable
    is DocumentJobException.Invalid -> DocumentFailureKind.Invalid
    is DocumentJobException.Limit -> DocumentFailureKind.Limit
    is DocumentJobException.Busy -> DocumentFailureKind.Busy
    is DocumentJobException.Cancelled -> DocumentFailureKind.Cancelled
    is DocumentJobException.Timeout -> DocumentFailureKind.Timeout
    is DocumentJobException.Isolation -> DocumentFailureKind.Isolation
    is DocumentJobException.Verification -> DocumentFailureKind.Verification
    is DocumentJobException.Password -> DocumentFailureKind.Password
    is DocumentJobException.Unsupported -> DocumentFailureKind.Unsupported
    is DocumentJobException.Malformed -> DocumentFailureKind.Malformed
    is DocumentJobException.Worker -> DocumentFailureKind.Worker
})

private fun Rect.jsonRegion(): JsonArray = JsonArray(listOf(x,y,x+width,y+height).map(::JsonPrimitive))
private fun Transform.jsonTransform(): JsonArray = JsonArray(listOf(a,b,c,d,e,f).map(::JsonPrimitive))
@OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)
public fun documentRequestJson(request: DocumentRequest): String {
    request.exportIdentity?.let { identity ->
        identity.validate()
        if (request.operation !is DocumentOperation.Copy) throw DocumentFailure(DocumentFailureKind.Invalid)
    }
    if (request.operation is DocumentOperation.Copy && request.exportIdentity == null)
        throw DocumentFailure(DocumentFailureKind.Invalid)
    if (request.password?.let { it.toByteArray(Charsets.UTF_8).size > 1024 || '\u0000' in it } == true)
        throw DocumentFailure(DocumentFailureKind.Invalid)
    (request.operation as? DocumentOperation.Copy)?.let { copy ->
        if (copy.overlays.size > 4096 || copy.regions.size > 4096) throw DocumentFailure(DocumentFailureKind.Limit)
        var contours = 0L;var points = 0L
        for (overlay in copy.overlays) {
            contours += overlay.contours.size
            if (contours > 16384) throw DocumentFailure(DocumentFailureKind.Limit)
            for (contour in overlay.contours) {
                points += contour.size
                if (points > 250000) throw DocumentFailure(DocumentFailureKind.Limit)
            }
        }
    }
    return buildJsonObject {
    put("limits",buildJsonObject { put("input_bytes",64 * 1024 * 1024);put("output_bytes",documentOutputLimit(request.operation));put("memory_bytes",512 * 1024 * 1024) })
    put("password",request.password?.let(::JsonPrimitive) ?: JsonNull)
    request.exportIdentity?.let { identity -> put("export_identity",buildJsonObject {
        put("schema_version",identity.schemaVersion.toLong());put("host_seq",JsonPrimitive(identity.hostSeq))
        put("state_hash",identity.stateHash);put("source_asset",identity.sourceAsset)
        identity.captureSession?.let { put("capture_session",it) }
        identity.frameId?.let { put("frame_id",JsonPrimitive(it)) }
    }) }
    put("operation",buildJsonObject {
        when(val op = request.operation) {
            DocumentOperation.PdfInfo -> put("kind","pdf_info")
            DocumentOperation.SvgInfo -> put("kind","svg_info")
            is DocumentOperation.Page -> { put("kind","pdf_page");put("page",op.index.toLong()) }
            is DocumentOperation.Tile -> {
                put("kind",if(op.kind == EditableKind.Pdf) "pdf_tile" else "svg_tile")
                if(op.kind == EditableKind.Pdf) { put("page",op.page.toLong());put("annotations",true) }
                put("tile",buildJsonObject {put("x",op.tile.origin.x);put("y",op.tile.origin.y);put("scale",op.tile.scale);put("width",op.tile.width.toLong());put("height",op.tile.height.toLong())})
            }
            is DocumentOperation.Text -> {put("kind","pdf_text");put("page",op.page.toLong());put("start",op.start.toLong());put("count",op.count.toLong())}
            is DocumentOperation.Selection -> {put("kind","pdf_selection");put("page",op.page.toLong());put("rectangle",op.rectangle.jsonRegion())}
            is DocumentOperation.Copy -> {
                put("kind",when(op.kind) {DocumentCopy.SvgAnnotations -> "svg_copy";DocumentCopy.PcRedaction -> "pdf_pc_redaction";DocumentCopy.EveryPageRasterRedaction -> "pdf_raster_redaction";else -> "pdf_copy"})
                if(op.kind in setOf(DocumentCopy.PcRedaction,DocumentCopy.EveryPageRasterRedaction)) {
                    put("scale",op.scale);put("explicitly_remove_encryption",op.explicitlyRemoveEncryption)
                    put("regions",JsonArray(op.regions.map{buildJsonObject{put("page",it.page.toLong());put("rectangle",it.rectangle.jsonRegion())}}))
                } else {
                    if(op.kind != DocumentCopy.SvgAnnotations) put("copy",when(op.kind){DocumentCopy.Original -> "original";DocumentCopy.Annotated -> "annotated";else -> "flattened"})
                    put("overlays",JsonArray(op.overlays.map{item -> buildJsonObject{
                        put("page",item.page.toLong());put("transform",item.transform.jsonTransform())
                        put("rgba",JsonArray(listOf((item.rgba shr 24).toInt() and 255,(item.rgba shr 16).toInt() and 255,(item.rgba shr 8).toInt() and 255,item.rgba.toInt() and 255).map(::JsonPrimitive)))
                        put("contours",JsonArray(item.contours.map{contour -> JsonArray(contour.map{JsonArray(listOf(JsonPrimitive(it.x),JsonPrimitive(it.y)))})}))
                    }}))
                }
            }
        }
    })
}.toString().also { if(it.toByteArray(Charsets.UTF_8).size > 65536) throw DocumentFailure(DocumentFailureKind.Limit) }
}

/** Called only after native typed result validation. No arbitrary JSON defaults. */
public fun documentReply(result: DocumentJobResult): DocumentReply = try {
    if(result.metadataJson.length > 1024 * 1024 || result.bytes.size > 128 * 1024 * 1024 || !result.sourceBlake3.matches(Regex("[0-9a-f]{64}")) || !result.outputBlake3.matches(Regex("[0-9a-f]{64}"))) throw DocumentFailure(DocumentFailureKind.Verification)
    val data = Json.parseToJsonElement(result.metadataJson).jsonObject
    fun n(key: String): Double = data.getValue(key).jsonPrimitive.double
    fun u(key: String): UInt = data.getValue(key).jsonPrimitive.long.toUInt()
    fun strings(): List<String> = data["notices"]?.jsonArray?.map{it.jsonPrimitive.content} ?: emptyList()
    fun glyphs(): List<DocumentGlyph> = data.getValue("glyphs").jsonArray.map{value ->
        val glyph = value.jsonObject; val bounds = glyph["bounds"]?.takeUnless{it is JsonNull}?.jsonArray?.map{it.jsonPrimitive.double}
        DocumentGlyph(glyph.getValue("index").jsonPrimitive.long.toUInt(),glyph["unicode"]?.jsonPrimitive?.contentOrNull,bounds?.let{Rect(it[0],it[1],it[2]-it[0],it[3]-it[1])},glyph.getValue("generated").jsonPrimitive.boolean)
    }
    fun exportIdentity(): DocumentExportIdentity? = data["export_identity"]?.takeUnless { it is JsonNull }?.jsonObject?.let { value ->
        if (value.keys.any { it !in setOf("schema_version","host_seq","state_hash","source_asset","capture_session","frame_id") })
            throw DocumentFailure(DocumentFailureKind.Verification)
        DocumentExportIdentity(value.getValue("schema_version").jsonPrimitive.content.toUInt(),
            value.getValue("host_seq").jsonPrimitive.content.toULong(),value.getValue("state_hash").jsonPrimitive.content,
            value.getValue("source_asset").jsonPrimitive.content,value["capture_session"]?.jsonPrimitive?.contentOrNull,
            value["frame_id"]?.jsonPrimitive?.contentOrNull?.toULong()).also { it.validate() }
    }
    val kind = data.getValue("kind").jsonPrimitive.content
    val meta: DocumentMetadata = when(kind) {
        "pdf_info" -> DocumentMetadata.Info(EditableKind.Pdf,u("pages"),null,null,data.getValue("encrypted").jsonPrimitive.boolean,strings())
        "svg_info" -> DocumentMetadata.Info(EditableKind.Svg,1u,n("width"),n("height"),false,strings())
        "pdf_page" -> DocumentMetadata.Page(u("page"),n("width"),n("height"),data.getValue("crop").jsonArray.map{it.jsonPrimitive.double},u("quarter_turns"))
        "rgba_tile" -> { val origin=data.getValue("origin").jsonArray; DocumentMetadata.Tile(data["page"]?.jsonPrimitive?.longOrNull?.toUInt(),Point(origin[0].jsonPrimitive.double,origin[1].jsonPrimitive.double),n("native_scale"),u("width"),u("height")) }
        "pdf_text","pdf_selection" -> DocumentMetadata.Text(u("page"),glyphs(),data["next"]?.jsonPrimitive?.longOrNull?.toUInt())
        "pdf_copy","svg_copy","raster_redaction","pc_redaction" -> DocumentMetadata.Copy(when(kind){"svg_copy"->DocumentCopy.SvgAnnotations;"raster_redaction"->DocumentCopy.EveryPageRasterRedaction;"pc_redaction"->DocumentCopy.PcRedaction;else->when(data.getValue("copy").jsonPrimitive.content){"original"->DocumentCopy.Original;"annotated"->DocumentCopy.Annotated;"flattened"->DocumentCopy.Flattened;else->throw DocumentFailure(DocumentFailureKind.Verification)}},data.getValue("bytes").jsonPrimitive.long.toULong(),kind in setOf("raster_redaction","pc_redaction"),strings(),data["plan_blake3"]?.jsonPrimitive?.contentOrNull,exportIdentity())
        else -> throw DocumentFailure(DocumentFailureKind.Verification)
    }
    DocumentReply(result.sourceBlake3,result.outputBlake3,meta,result.bytes)
} catch (error: DocumentFailure) { throw error }
catch (_: Exception) { throw DocumentFailure(DocumentFailureKind.Verification) }

internal suspend fun <T> createDocumentNative(options: CreateDocument, wrap: (ProjectSession) -> T): T {
    val handle = settledStream(release = { value: ProjectSession -> try { value.closeSession() } finally { value.destroy() } }) { cancel ->
        createEditableDocument(options.documentNative(), cancel)
    }
    try { return wrap(handle) }
    catch (error: Throwable) { withContext(NonCancellable) { try { handle.closeSession() } finally { handle.destroy() } }; throw error }
}
