package com.visualworkbench.shared

import kotlinx.coroutines.runBlocking
import kotlin.test.*

class DocumentInteractionTest {
    private val asset = "a".repeat(64)
    private val revision = ProjectInfo("project","title","device",1uL,false,false,1uL,"b".repeat(64),listOf("doc"))
    private val core = object : WorkbenchCore {
        private var id = 0
        override fun newId(unixMs: ULong): String = "layer-${id++}"
        override fun newDeviceId(): String = "device"
        override fun cameraMatrix(camera: Camera,inverse: Boolean): Transform = Transform()
        override fun mapPoints(camera: Camera,inverse: Boolean,points: List<Point>): List<Point> = points
        override suspend fun create(options: CreateProject): WorkbenchProject = error("unused")
        override suspend fun open(path: String): WorkbenchProject = error("unused")
        override suspend fun layoutText(text: String,font: String,size: Float): TextLayout = error("unused")
    }
    private class Jobs(val response: (DocumentRequest) -> DocumentReply) : DocumentJobs {
        override val pcRedaction: Boolean = false
        var calls = 0
        override suspend fun run(request: DocumentRequest,source: ByteArray): DocumentReply { calls++;return response(request) }
        override suspend fun close(): Unit = Unit
    }
    private fun info(kind: EditableKind = EditableKind.Svg,pages: UInt = 1u,width: Double = 12.5): DocumentReply =
        DocumentReply(asset,"c".repeat(64),DocumentMetadata.Info(kind,pages,width,20.0,false,emptyList()),byteArrayOf())
    private fun source(): EditableSource = EditableSource(EditableDescriptor("doc",asset,EditableKind.Svg,
        listOf(DocumentPage(0u,12.5,20.0,listOf(0.0,0.0,12.5,20.0),0u,"layer")),revision),byteArrayOf(1))
    private val request = DocumentTileRequest(Point(0.0,0.0),1.25,1u,1u)
    private fun tile(meta: DocumentMetadata.Tile = DocumentMetadata.Tile(null,request.origin,1.25,1u,1u),bytes: ByteArray = byteArrayOf(1,2,3,-1),source: String = asset): DocumentReply =
        DocumentReply(source,"c".repeat(64),meta,bytes)

    @Test fun fractionalSvgGeometryAndOriginalIdentityArePreserved(): Unit = runBlocking {
        val jobs=Jobs { info() }
        val (reply,pages)=DocumentInteraction(jobs,core).inspect(byteArrayOf(1),EditableKind.Svg)
        assertEquals(asset,reply.sourceBlake3);assertEquals(12.5,pages.single().width);assertEquals(1,jobs.calls)
    }
    @Test fun svgCannotInventExtraPagesOrNonfiniteGeometry(): Unit = runBlocking {
        for(reply in listOf(info(pages=2u),info(width=Double.NaN),info(width=65_536.0)))
            assertEquals(DocumentFailureKind.Verification,assertFailsWith<DocumentFailure> {
                DocumentInteraction(Jobs { reply },core).inspect(byteArrayOf(1),EditableKind.Svg)
            }.kind)
    }
    @Test fun pdfPageReplyCannotChangeTheOriginal(): Unit = runBlocking {
        val jobs=Jobs { if(it.operation==DocumentOperation.PdfInfo)info(EditableKind.Pdf) else
            DocumentReply("d".repeat(64),"c".repeat(64),DocumentMetadata.Page(0u,12.0,20.0,listOf(0.0,0.0,12.0,20.0),0u),byteArrayOf()) }
        assertEquals(DocumentFailureKind.Verification,assertFailsWith<DocumentFailure> {
            DocumentInteraction(jobs,core).inspect(byteArrayOf(1),EditableKind.Pdf)
        }.kind)
    }
    @Test fun tileKeepsExactPixelsWithoutColorOrCoordinateSubstitution(): Unit = runBlocking {
        val response=tile()
        assertSame(response,DocumentInteraction(Jobs { response },core).tile(source(),0u,request))
    }
    @Test fun tileSourceAndPageMustMatchTheRequest(): Unit = runBlocking {
        for(reply in listOf(tile(source="d".repeat(64)),tile(DocumentMetadata.Tile(0u,request.origin,1.25,1u,1u))))
            assertEquals(DocumentFailureKind.Verification,assertFailsWith<DocumentFailure> {
                DocumentInteraction(Jobs { reply },core).tile(source(),0u,request)
            }.kind)
    }
    @Test fun tileOriginScaleDimensionsAndPayloadMustMatch(): Unit = runBlocking {
        for(reply in listOf(tile(DocumentMetadata.Tile(null,Point(1.0,0.0),1.25,1u,1u)),
            tile(DocumentMetadata.Tile(null,request.origin,2.0,1u,1u)),tile(DocumentMetadata.Tile(null,request.origin,1.25,2u,1u)),tile(bytes=byteArrayOf(1))))
            assertEquals(DocumentFailureKind.Verification,assertFailsWith<DocumentFailure> {
                DocumentInteraction(Jobs { reply },core).tile(source(),0u,request)
            }.kind)
    }
    @Test fun svgTextSelectionIsRefusedBeforeDispatch(): Unit = runBlocking {
        val jobs=Jobs { error("must not dispatch") }
        assertEquals(DocumentFailureKind.Unsupported,assertFailsWith<DocumentFailure> {
            DocumentInteraction(jobs,core).select(source(),0u,Rect(0.0,0.0,1.0,1.0))
        }.kind)
        assertEquals(0,jobs.calls)
    }
}
