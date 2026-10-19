package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.McpInboxReceipt
import com.visualworkbench.shared.PackageInfo

/** Existing immutable inbox identity, not a claim of registered image alignment. */
internal data class McpMarkerReviewKey(
    val original: McpVerificationPackage, val receiptId: String,
    val receiptBlake3: String, val afterBlake3: String,
)
internal enum class McpMarkerVerdict { Pending, Pass, Fail }
internal data class McpMarkerAssessment(val originalNumber: UInt, val verdict: McpMarkerVerdict, val note: String)
internal data class McpMarkerReviewSnapshot(
    val key: McpMarkerReviewKey, val version: ULong, val markers: List<McpMarkerAssessment>,
)
/** Input selection for the existing compiler, not a new package format. Original
 * numbers remain explicit even if a follow-up compiler uses local numbering. */
internal data class McpFailureSelection(
    val key: McpMarkerReviewKey, val reviewVersion: ULong, val failed: List<McpMarkerAssessment>,
)

/** Manual owner decisions for existing Compare returns. Nothing here derives a
 * verdict from pixels, auto-accepts a result, writes a project or publishes.
 * Persistence and showing an aligned pair remain coordinator composition. */
internal class McpMarkerAssessments private constructor(private val key: McpMarkerReviewKey) {
    private var version = 0uL
    private val markers = Array(key.original.markerCount.toInt()) { index ->
        McpMarkerAssessment(index.toUInt() + 1u, McpMarkerVerdict.Pending, "")
    }
    private var closed = false
    private fun current(expected: McpMarkerReviewSnapshot) {
        check(!closed) { "Marker review closed" }
        require(expected.key == key && expected.version == version) { "Marker review changed" }
    }
    private fun <T> immutable(values: Collection<T>): List<T> = java.util.Collections.unmodifiableList(ArrayList(values))

    @Synchronized fun snapshot(): McpMarkerReviewSnapshot {
        check(!closed) { "Marker review closed" }
        return McpMarkerReviewSnapshot(key, version, immutable(markers.asList()))
    }

    /** Invoke from an explicit owner UI action, never an MCP-agent callback.
     * Expected pair/version fences delayed UI events and reopened receipts. */
    @Synchronized fun record(expected: McpMarkerReviewSnapshot, number: UInt, verdict: McpMarkerVerdict,
        note: String = ""): McpMarkerReviewSnapshot {
        current(expected)
        require(number >= 1u && number <= markers.size.toUInt())
        require(note.length <= 32768 && '\u0000' !in note)
        val bytes = try {
            Charsets.UTF_8.newEncoder().onMalformedInput(java.nio.charset.CodingErrorAction.REPORT)
                .onUnmappableCharacter(java.nio.charset.CodingErrorAction.REPORT).encode(java.nio.CharBuffer.wrap(note)).remaining()
        } catch (_: java.nio.charset.CharacterCodingException) {
            throw IllegalArgumentException("Marker note UTF-8 invalid")
        }
        require(bytes <= 32768)
        val index = number.toInt() - 1
        val total = markers.indices.sumOf { if (it == index) bytes else markers[it].note.toByteArray(Charsets.UTF_8).size }
        require(total <= 128 * 1024)
        check(version != ULong.MAX_VALUE)
        markers[index] = McpMarkerAssessment(number, verdict, note)
        version++
        return snapshot()
    }

    /** Refuse to silently discard unreviewed markers. An all-pass review has
     * no follow-up, rather than a new package with no failed instructions. */
    @Synchronized fun failures(expected: McpMarkerReviewSnapshot): McpFailureSelection? {
        current(expected)
        check(markers.none { it.verdict == McpMarkerVerdict.Pending }) { "Markers await owner review" }
        val failed = markers.filter { it.verdict == McpMarkerVerdict.Fail }
        return if (failed.isEmpty()) null else McpFailureSelection(key, version, immutable(failed))
    }

    /** Recheck immediately before preparing/publishing a follow-up. A newer
     * owner decision cannot reuse an earlier failed-marker selection. */
    @Synchronized fun checkCurrent(selection: McpFailureSelection) {
        val expected = snapshot()
        require(selection == failures(expected)) { "Failure selection changed" }
    }

    @Synchronized fun close() {
        closed = true
        for (index in markers.indices) markers[index] = markers[index].copy(note = "", verdict = McpMarkerVerdict.Pending)
    }

    companion object {
        fun open(original: PackageInfo, receipt: McpInboxReceipt): McpMarkerAssessments {
            val source = McpVerificationPackage.of(original)
            require(!receipt.retired && receipt.packageId == source.packageId && receipt.target == source.target &&
                receipt.manifestSha256 == source.manifestSha256 && receipt.binding == source.binding &&
                receipt.sourceAssetId == source.sourceAssetId)
            val after = requireNotNull(receipt.after)
            val hash = Regex("[0-9a-f]{64}")
            require(hash.matches(receipt.receiptBlake3) && hash.matches(after.blake3))
            require(receipt.receiptId.matches(Regex("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")))
            val clean = original.images.single { it.role == "clean_source" }
            require(clean.width == receipt.before.width && clean.height == receipt.before.height)
            return McpMarkerAssessments(McpMarkerReviewKey(source, receipt.receiptId, receipt.receiptBlake3, after.blake3))
        }
    }
}
