package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.PackageInfo
import com.visualworkbench.shared.WorkflowBinding

/** Immutable subset of the existing checked PackageInfo. No paths or OS targets. */
internal data class McpVerificationPackage(
    val packageId: String, val target: String, val manifestSha256: String,
    val binding: WorkflowBinding, val sourceAssetId: String, val markerCount: UInt,
) {
    companion object {
        fun of(info: PackageInfo): McpVerificationPackage {
            val uuid = Regex("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
            val hash = Regex("[0-9a-f]{64}")
            require(uuid.matches(info.packageId) && uuid.matches(info.binding.projectId) && uuid.matches(info.binding.documentId))
            require(hash.matches(info.manifestSha256) && hash.matches(info.binding.stateHash) && hash.matches(info.sourceAssetId))
            require(info.target in setOf("claude", "openai", "gemini", "generic") && info.markerCount <= 512u)
            return McpVerificationPackage(info.packageId, info.target, info.manifestSha256,
                info.binding.copy(), info.sourceAssetId, info.markerCount)
        }
    }
}

/** Association only: this records the owner's exact capture route, not image
 * alignment, physical capture acceptance, a grant or an exterior-pixel proof. */
internal data class McpVerificationRecapture(
    val original: McpVerificationPackage, val recaptured: McpVerificationPackage,
)

/** Session-only relation. The native engine remains the sole selector -> exact
 * CaptureTarget owner. Never serialize this object or restore its selectors.
 * All methods are synchronized; no lock is retained during capture or IO. */
internal class McpVerificationTargets {
    private data class Source(val project: String, val document: String, val asset: String)
    private class Origin(val selector: String)
    private class Bound(val source: McpVerificationPackage, val origin: Origin)
    internal class Ticket internal constructor(
        internal val original: McpVerificationPackage, internal val selector: String,
    )
    private val origins = linkedMapOf<Source, Origin>()
    private val packages = linkedMapOf<Pair<String, String>, Bound>()
    private val pending = java.util.IdentityHashMap<Ticket, Bound>()
    private var closed = false
    private fun live() { check(!closed) { "Verification session closed" } }
    private fun source(info: McpVerificationPackage) = Source(info.binding.projectId, info.binding.documentId, info.sourceAssetId)
    private fun key(info: McpVerificationPackage) = info.packageId to info.target

    /** Owner calls only after the existing native compile for this selector
     * settles. Package metadata supplied by an agent cannot establish a route. */
    @Synchronized fun rememberCapture(info: PackageInfo, selector: String) {
        live(); require(selector.matches(Regex("[a-zA-Z0-9_-]{1,64}")))
        val identity = McpVerificationPackage.of(info)
        val source = source(identity)
        val previous = origins[source]
        // An identical image in another selected window is not target proof.
        require(previous == null || previous.selector == selector)
        if (previous == null) {
            check(origins.size < 64) { "Verification source limit" }
            origins[source] = Origin(selector)
        }
    }

    /** A later marked revision of the same canonical source can be bound after
     * catalog publication. Unknown imports remain available without recapture. */
    @Synchronized fun rememberPackage(info: PackageInfo): Boolean {
        live(); val identity = McpVerificationPackage.of(info)
        val previous = packages[key(identity)]
        if (previous != null) { require(previous.source == identity); return true }
        val origin = origins[source(identity)] ?: return false
        check(packages.size < 64) { "Verification package limit" }
        packages[key(identity)] = Bound(identity, origin)
        return true
    }

    /** No capture permission is minted. Caller must still use the existing
     * exact connection/selector grant and indicator/cancellation path. */
    @Synchronized fun begin(info: PackageInfo): Ticket {
        live(); val identity = McpVerificationPackage.of(info)
        val bound = packages[key(identity)] ?: error("Original target unavailable")
        require(bound.source == identity)
        check(pending.size < 4) { "Verification recapture limit" }
        return Ticket(identity, bound.origin.selector).also { pending[it] = bound }
    }

    @Synchronized fun checkCurrent(ticket: Ticket) {
        live(); val bound = pending[ticket] ?: error("Verification ticket unavailable")
        check(packages[key(ticket.original)] === bound && origins[source(ticket.original)] === bound.origin) {
            "Verification original retired"
        }
    }

    /** Owner calls only with the exact package published by the joined compile
     * for ticket.selector. This method does not infer alignment from dimensions. */
    @Synchronized fun complete(ticket: Ticket, after: PackageInfo): McpVerificationRecapture {
        try {
            checkCurrent(ticket)
            val result = McpVerificationPackage.of(after)
            require(result.packageId != ticket.original.packageId)
            return McpVerificationRecapture(ticket.original, result)
        } finally { pending.remove(ticket) }
    }

    /** Call after joined cancellation/failed publication; cannot release any
     * native worker because this registry owns only immutable routing data. */
    @Synchronized fun abandon(ticket: Ticket) { pending.remove(ticket) }

    /** Invalidate before the existing unpublish/drain; late completions refuse.
     * Outstanding tickets retain admission until their actual owner settles. */
    @Synchronized fun retire(info: PackageInfo) {
        live(); val identity = McpVerificationPackage.of(info)
        val bound = packages[key(identity)] ?: return
        require(bound.source == identity)
        packages.remove(key(identity))
    }

    /** Coordinator joins its captures first, as for its existing native owners. */
    @Synchronized fun close() {
        closed = true; packages.clear(); origins.clear(); pending.clear()
    }
}
