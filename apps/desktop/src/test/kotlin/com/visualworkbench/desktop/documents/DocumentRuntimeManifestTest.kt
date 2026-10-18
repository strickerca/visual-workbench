package com.visualworkbench.desktop.documents

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

class DocumentRuntimeManifestTest {
    private fun line(name: String, size: String = "1") = "${"a".repeat(64)} $size $name\n"
    private fun failure(kind: DocumentFailureKind, block: () -> Unit) {
        try { block(); fail("expected fixed failure") } catch (error: DocumentFailure) { assertEquals(kind, error.kind) }
    }
    @Test fun svgWorkerManifestDoesNotNeedAnUnrelatedPdfiumArtifact() {
        val manifest = DocumentRuntimeManifest.parse(line("vw-document-worker.exe").toByteArray())
        assertFalse(manifest.pdfium); assertFalse(manifest.pcRedaction); assertEquals(64, manifest.key.length)
    }
    @Test fun qpdfRequiresPdfiumAndExactClosedResourceNames() {
        val all = line("vw-document-worker.exe") + line("pdfium.dll") + line("qpdf30.dll")
        assertTrue(DocumentRuntimeManifest.parse(all.toByteArray()).pcRedaction)
        failure(DocumentFailureKind.Verification) { DocumentRuntimeManifest.parse((line("vw-document-worker.exe") + line("qpdf30.dll")).toByteArray()) }
        for (name in listOf("../pdfium.dll", "PDFIUM.DLL", "system.dll", "qpdf.exe"))
            failure(DocumentFailureKind.Verification) { DocumentRuntimeManifest.parse((line("vw-document-worker.exe") + line(name)).toByteArray()) }
    }
    @Test fun duplicateMalformedAndOversizedEntriesRefuse() {
        failure(DocumentFailureKind.Verification) { DocumentRuntimeManifest.parse((line("vw-document-worker.exe").repeat(2)).toByteArray()) }
        failure(DocumentFailureKind.Verification) { DocumentRuntimeManifest.parse(line("vw-document-worker.exe", "0").toByteArray()) }
        failure(DocumentFailureKind.Limit) { DocumentRuntimeManifest.parse(line("vw-document-worker.exe", "268435457").toByteArray()) }
        failure(DocumentFailureKind.Verification) { DocumentRuntimeManifest.parse(ByteArray(8193) { 32 }) }
        failure(DocumentFailureKind.Verification) { DocumentRuntimeManifest.parse(line("vw-document-worker.exe").replace('\n', '\r').toByteArray()) }
    }
    @Test fun everyLeaseIsAttemptedAndOnlyFailedOwnersAreRetried() {
        val calls = mutableListOf<Int>(); var fail = true
        val leases = DocumentRuntimeLeases(mutableListOf(AutoCloseable { calls += 1 }, AutoCloseable { calls += 2; if (fail) error("synthetic") }, AutoCloseable { calls += 3 }))
        failure(DocumentFailureKind.Isolation) { leases.close() }; assertEquals(listOf(3, 2, 1), calls)
        fail = false; leases.close(); leases.close(); assertEquals(listOf(3, 2, 1, 2), calls)
    }
}
