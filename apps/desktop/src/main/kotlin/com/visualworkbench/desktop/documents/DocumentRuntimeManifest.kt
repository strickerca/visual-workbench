package com.visualworkbench.desktop.documents

import com.visualworkbench.shared.*
import java.security.MessageDigest

internal data class DocumentArtifact(val name: String, val size: Long, val sha256: String)
internal class DocumentRuntimeManifest private constructor(val entries: List<DocumentArtifact>, val key: String) {
    val pdfium: Boolean get() = entries.any { it.name == "pdfium.dll" }
    val pcRedaction: Boolean get() = entries.any { it.name == "qpdf30.dll" }

    companion object {
        private val entry = Regex("([0-9a-f]{64}) ([1-9][0-9]{0,8}) (pdfium\\.dll|qpdf30\\.dll|vw-document-worker\\.exe)")
        fun parse(bytes: ByteArray): DocumentRuntimeManifest {
            if (bytes.size !in 1..8192 || bytes.any { it.toInt() !in 32..126 && it.toInt() != 10 })
                throw DocumentFailure(DocumentFailureKind.Verification)
            val entries = bytes.toString(Charsets.US_ASCII).lineSequence().filter(String::isNotEmpty).map { line ->
                val match = entry.matchEntire(line) ?: throw DocumentFailure(DocumentFailureKind.Verification)
                val size = match.groupValues[2].toLong()
                if (size > 256L * 1024 * 1024) throw DocumentFailure(DocumentFailureKind.Limit)
                DocumentArtifact(match.groupValues[3], size, match.groupValues[1])
            }.toList()
            if (entries.size !in 1..3 || entries.map { it.name }.toSet().size != entries.size ||
                entries.none { it.name == "vw-document-worker.exe" } ||
                entries.any { it.name == "qpdf30.dll" } && entries.none { it.name == "pdfium.dll" })
                throw DocumentFailure(DocumentFailureKind.Verification)
            return DocumentRuntimeManifest(entries, documentHex(MessageDigest.getInstance("SHA-256").digest(bytes)))
        }
    }
}

internal fun documentHex(bytes: ByteArray): String = bytes.joinToString("") { "%02x".format(it.toInt() and 255) }
internal fun documentUnhex(text: String): ByteArray = text.chunked(2).map { it.toInt(16).toByte() }.toByteArray()

/** Retire every lease, retaining only failed leases for close retry. */
internal class DocumentRuntimeLeases(private val retained: MutableList<AutoCloseable>) : AutoCloseable {
    override fun close() {
        var failed = false
        for (index in retained.indices.reversed()) {
            try { retained[index].close(); retained.removeAt(index) }
            catch (_: Exception) { failed = true }
        }
        if (failed) throw DocumentFailure(DocumentFailureKind.Isolation)
    }
}
