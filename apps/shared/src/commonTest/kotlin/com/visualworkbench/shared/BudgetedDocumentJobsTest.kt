package com.visualworkbench.shared

import kotlinx.coroutines.runBlocking
import kotlin.test.*

class BudgetedDocumentJobsTest {
    private class Jobs:DocumentJobs {
        override val pcRedaction=true
        var calls=0;var closed=false
        override suspend fun run(request:DocumentRequest,source:ByteArray):DocumentReply{calls++;return DocumentReply("a".repeat(64),"b".repeat(64),DocumentMetadata.Info(EditableKind.Pdf,1u,null,null,false,emptyList()),byteArrayOf())}
        override suspend fun close(){closed=true}
    }
    @Test fun retainedDisplayAndForeignSourceCopiesAreAdmittedBeforeTransport():Unit=runBlocking {
        val jobs=Jobs();val owner=BudgetedDocumentJobs(jobs,{128L*1024*1024})
        assertEquals(DocumentFailureKind.Limit,assertFailsWith<DocumentFailure>{owner.run(DocumentRequest(DocumentOperation.PdfInfo),ByteArray(24*1024*1024))}.kind)
        assertEquals(0,jobs.calls)
        owner.run(DocumentRequest(DocumentOperation.PdfInfo),byteArrayOf(1));assertEquals(1,jobs.calls)
    }
    @Test fun invalidTileDimensionsCannotOverflowAParentAdmissionEstimate():Unit=runBlocking {
        val jobs=Jobs();val owner=BudgetedDocumentJobs(jobs,{0})
        for(side in listOf(0u,513u,UInt.MAX_VALUE))assertEquals(DocumentFailureKind.Invalid,assertFailsWith<DocumentFailure>{owner.run(DocumentRequest(DocumentOperation.Tile(EditableKind.Svg,0u,DocumentTileRequest(Point(0.0,0.0),1.0,side,side))),byteArrayOf(1))}.kind)
        assertEquals(0,jobs.calls)
    }
    @Test fun isolatedCapabilityAndJoinedCloseBelongToTheUnderlyingOwner():Unit=runBlocking {
        val jobs=Jobs();val owner=BudgetedDocumentJobs(jobs,{0});assertTrue(owner.pcRedaction);owner.close();assertTrue(jobs.closed)
    }
}
