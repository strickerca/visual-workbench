package com.visualworkbench.android.documents

import com.visualworkbench.shared.*
import java.io.*
import org.junit.Assert.*
import org.junit.Test

class DocumentWireTest {
    private fun result(bytes: ByteArray = byteArrayOf(1, 2, 3)) = DocumentWireResult("a".repeat(64), "{}", bytes, "b".repeat(64))
    private fun encoded(value: DocumentWireResult? = result(), failure: DocumentFailureKind? = null): ByteArray = ByteArrayOutputStream().also { DocumentWire.write(it, value, failure) }.toByteArray()
    private fun failure(kind: DocumentFailureKind, block: () -> Unit) {
        try { block(); fail("expected fixed failure") } catch (error: DocumentFailure) { assertEquals(kind, error.kind); assertEquals(kind.name, error.message) }
    }
    @Test fun exactSuccessRoundTripAndPayloadCap() {
        val decoded = DocumentWire.read(encoded().inputStream(), 3)
        assertEquals("a".repeat(64), decoded.sourceBlake3); assertEquals("{}", decoded.metadataJson)
        assertArrayEquals(byteArrayOf(1, 2, 3), decoded.bytes)
        failure(DocumentFailureKind.Limit) { DocumentWire.read(encoded().inputStream(), 2) }
    }
    @Test fun closedFailuresRoundTripWithoutRawCauses() {
        for (kind in DocumentFailureKind.entries.filter { it != DocumentFailureKind.Stale }) {
            failure(kind) { DocumentWire.read(encoded(null, kind).inputStream()) }
        }
    }
    @Test fun trailingBytesNeverPassOnSuccessOrFailure() {
        for (bytes in listOf(encoded(), encoded(null, DocumentFailureKind.Password)))
            failure(DocumentFailureKind.Verification) { DocumentWire.read((bytes + byteArrayOf(7)).inputStream()) }
    }
    @Test fun truncatedMagicHeaderMetadataAndPayloadRefuse() {
        val bytes = encoded()
        for (length in listOf(0, 7, 8, 11, 16, bytes.size - 1))
            failure(DocumentFailureKind.Verification) { DocumentWire.read(bytes.copyOf(length).inputStream()) }
    }
    @Test fun unknownStatusAndFailureCodesAreNotOrdinaryWorkerFailures() {
        val status = encoded(); status[11] = 2
        failure(DocumentFailureKind.Verification) { DocumentWire.read(status.inputStream()) }
        val output = ByteArrayOutputStream(); val data = DataOutputStream(output)
        data.write(encoded().copyOf(8)); data.writeInt(1); data.writeInt(7); data.writeBytes("unknown")
        failure(DocumentFailureKind.Verification) { DocumentWire.read(output.toByteArray().inputStream()) }
    }
    @Test fun invalidUtf8AndOversizedStringHeadersRefuseBeforePayloadRead() {
        for ((length, bytes, kind) in listOf(Triple(1, byteArrayOf(0xff.toByte()), DocumentFailureKind.Verification), Triple(Int.MAX_VALUE, byteArrayOf(), DocumentFailureKind.Limit))) {
            val output = ByteArrayOutputStream(); val data = DataOutputStream(output)
            data.write(encoded().copyOf(8)); data.writeInt(1); data.writeInt(length); data.write(bytes)
            failure(kind) { DocumentWire.read(output.toByteArray().inputStream()) }
        }
    }
    @Test fun oneTerminalRecordAndWellFormedDigestsAreRequiredBeforeWriting() {
        val output = ByteArrayOutputStream()
        failure(DocumentFailureKind.Verification) { DocumentWire.write(output, result(), DocumentFailureKind.Worker) }
        assertEquals(0, output.size())
        failure(DocumentFailureKind.Verification) { DocumentWire.write(output, DocumentWireResult("private", "{}", byteArrayOf(), "b".repeat(64)), null) }
        assertEquals(0, output.size())
    }
    @Test fun sourceIsBoundedAndZeroProgressCannotSpin() {
        assertArrayEquals(byteArrayOf(1, 2), DocumentWire.source(byteArrayOf(1, 2).inputStream(), 2))
        failure(DocumentFailureKind.Limit) { DocumentWire.source(byteArrayOf(1, 2, 3).inputStream(), 2) }
        failure(DocumentFailureKind.Invalid) { DocumentWire.source(byteArrayOf().inputStream(), 2) }
        val zero = object : InputStream() { override fun read() = 0; override fun read(bytes: ByteArray, offset: Int, length: Int) = 0 }
        failure(DocumentFailureKind.Verification) { DocumentWire.source(zero, 2) }
    }
}
