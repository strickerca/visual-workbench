package com.visualworkbench.shared

import com.visualworkbench.bindings.core.DocumentJobResult
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

/** Actual JVM/native JSON adapter. Integer identity must never cross a signed
 * Long conversion or become a JSON string at the worker boundary. */
class DocumentWireIdentityTest {
    private val identity=DocumentExportIdentity(1u,ULong.MAX_VALUE,"b".repeat(64),"a".repeat(64),
        "018bcfe5-6800-7000-8000-000000000001",ULong.MAX_VALUE)
    private fun request():JsonObject=Json.parseToJsonElement(documentRequestJson(
        DocumentRequest(DocumentOperation.Copy(DocumentCopy.SvgAnnotations),exportIdentity=identity))).jsonObject
    @Test fun maximumUnsignedRevisionAndCaptureIdsKeepNumericJsonAndFullHashes() {
        val value=request();val fields=value.getValue("export_identity").jsonObject
        for(key in listOf("host_seq","frame_id")){
            assertEquals("18446744073709551615",fields.getValue(key).jsonPrimitive.content)
            assertFalse(fields.getValue(key).jsonPrimitive.isString)
        }
        assertEquals(identity.stateHash,fields.getValue("state_hash").jsonPrimitive.content)
        assertEquals(identity.sourceAsset,fields.getValue("source_asset").jsonPrimitive.content)
        assertEquals(8L*1024*1024,value.getValue("limits").jsonObject.getValue("output_bytes").jsonPrimitive.long)
    }
    @Test fun typedReplyKeepsExactIdentityAndRejectsAnUnknownCarrierField() {
        val fields=request().getValue("export_identity").jsonObject
        fun reply(value:JsonObject):DocumentReply {
            val metadata=buildJsonObject {put("kind","svg_copy");put("bytes",1);put("notices",JsonArray(emptyList()));put("export_identity",value)}
            return documentReply(DocumentJobResult(identity.sourceAsset,metadata.toString(),byteArrayOf(1),"c".repeat(64)))
        }
        assertEquals(identity,(reply(fields).metadata as DocumentMetadata.Copy).exportIdentity)
        val error=assertThrows(DocumentFailure::class.java){reply(JsonObject(fields+mapOf("unknown" to JsonPrimitive(true))))}
        assertEquals(DocumentFailureKind.Verification,error.kind)
    }
    @Test fun copiesCannotLeaveWithoutIdentityOrWithOnlyOneCaptureField() {
        val missing=assertThrows(DocumentFailure::class.java){documentRequestJson(DocumentRequest(DocumentOperation.Copy(DocumentCopy.SvgAnnotations)))}
        assertEquals(DocumentFailureKind.Invalid,missing.kind)
        val partial=assertThrows(DocumentFailure::class.java){documentRequestJson(DocumentRequest(DocumentOperation.Copy(DocumentCopy.SvgAnnotations),exportIdentity=identity.copy(frameId=null)))}
        assertEquals(DocumentFailureKind.Invalid,partial.kind)
    }
}
