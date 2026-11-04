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
    val kind: McpVerificationTargetKind = McpVerificationTargetKind.Window,
)

/** Session-only relation. The native engine remains the sole selector -> exact
 * CaptureTarget owner. Never serialize this object or restore its selectors.
 * All methods are synchronized; no lock is retained during capture or IO. */
internal class McpVerificationTargets {
    private data class Source(val project: String, val document: String, val asset: String)
    /** A phone-screen origin may have no live selector: a phone-app capture
     * until the owner binds a route, or a tunnel capture whose exact session
     * retired. `unbound` is the honest refusal reported in that state. */
    private class Origin(val selector: String?, val kind: McpVerificationTargetKind,
        val unbound: McpRecaptureRefusal = McpRecaptureRefusal.PhoneRouteUnbound)
    private class Bound(val source: McpVerificationPackage, val origin: Origin)
    internal class Ticket internal constructor(
        internal val original: McpVerificationPackage, internal val selector: String,
        internal val kind: McpVerificationTargetKind = McpVerificationTargetKind.Window,
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
    @Synchronized fun rememberCapture(info: PackageInfo, selector: String,
        kind: McpVerificationTargetKind = McpVerificationTargetKind.Window) {
        val identity = McpVerificationPackage.of(info)
        rememberSource(identity.binding,identity.sourceAssetId,selector,kind)
    }

    /** Existing Desktop capture delivery supplies this private association;
     * native retains its exact target before any marked revision is published. */
    @Synchronized fun rememberSource(binding:WorkflowBinding,asset:String,selector:String,
        kind:McpVerificationTargetKind=McpVerificationTargetKind.Window) {
        live(); require(selector.matches(Regex("[a-zA-Z0-9_-]{1,64}")))
        // Phone origins are created unbound and only an owner binding adds a route.
        require(kind != McpVerificationTargetKind.Phone)
        require(binding.projectId.matches(Regex("[0-9a-f-]{36}"))&&binding.documentId.matches(Regex("[0-9a-f-]{36}"))&&asset.matches(Regex("[0-9a-f]{64}")))
        val source = Source(binding.projectId,binding.documentId,asset)
        val previous = origins[source]
        // An identical image in another selected window/session is not target proof.
        require(previous == null || (previous.selector == selector && previous.kind == kind))
        if (previous == null) {
            check(origins.size < 64) { "Verification source limit" }
            origins[source] = Origin(selector, kind)
        }
    }

    /** A phone-screen capture made by the phone app reached this PC inside a
     * replicated project, so no PC capture route is known. Register it unbound;
     * the session link cannot request a phone screenshot. Returns whether the
     * source is (now) a Phone origin; an existing routed origin is never changed. */
    @Synchronized fun rememberPhoneSource(info: PackageInfo): Boolean {
        live(); val identity = McpVerificationPackage.of(info)
        val source = source(identity)
        val previous = origins[source]
        if (previous != null) return previous.kind == McpVerificationTargetKind.Phone
        check(origins.size < 64) { "Verification source limit" }
        origins[source] = Origin(null, McpVerificationTargetKind.Phone)
        return true
    }

    /** A tunnel capture whose exact session was no longer live when it could be
     * registered (session closed, or the local service was off at delivery). It
     * stays a tunnel capture that needs a reconnect and owner binding, never a
     * silently reclassified phone-app capture. */
    @Synchronized fun rememberUnboundTunnel(binding: WorkflowBinding, asset: String) {
        live()
        require(binding.projectId.matches(Regex("[0-9a-f-]{36}"))&&binding.documentId.matches(Regex("[0-9a-f-]{36}"))&&asset.matches(Regex("[0-9a-f]{64}")))
        val source = Source(binding.projectId, binding.documentId, asset)
        val previous = origins[source]
        if (previous != null) { require(previous.kind == McpVerificationTargetKind.Tunnel); return }
        check(origins.size < 64) { "Verification source limit" }
        origins[source] = Origin(null, McpVerificationTargetKind.Tunnel, McpRecaptureRefusal.PhoneDisconnected)
    }

    /** The engine forgot these routes because their exact sessions retired.
     * Their origins keep their kind but need a reconnect and owner binding.
     * Returns the affected bound package keys ("packageId/target"). */
    @Synchronized fun retireSelectors(selectors: Set<String>): Set<String> {
        live()
        val replaced = java.util.IdentityHashMap<Origin, Origin>()
        for ((source, origin) in origins.entries.toList()) {
            val selector = origin.selector ?: continue
            if (selector !in selectors) continue
            val next = Origin(null, origin.kind, McpRecaptureRefusal.PhoneDisconnected)
            replaced[origin] = next; origins[source] = next
        }
        if (replaced.isEmpty()) return emptySet()
        val affected = LinkedHashSet<String>()
        packages.replaceAll { key, value ->
            val next = replaced[value.origin] ?: return@replaceAll value
            affected += "${key.first}/${key.second}"
            Bound(value.source, next)
        }
        return affected
    }

    /** Explicit local owner action only: the owner attests that this live PC
     * route captures the same phone screen. Applies to phone-app captures and
     * to tunnel captures whose exact session was replaced; a window origin can
     * never be rebound. A new route makes the origin owner-bound (Phone), and
     * pending tickets for the earlier route are invalidated because the origin
     * object is replaced. Re-binding the same route is a no-op. */
    @Synchronized fun bindPhone(info: PackageInfo, selector: String) {
        live(); require(selector.matches(Regex("[a-zA-Z0-9_-]{1,64}")))
        val identity = McpVerificationPackage.of(info)
        val bound = packages[key(identity)] ?: error("Original target unavailable")
        require(bound.source == identity)
        val old = bound.origin
        require(old.kind != McpVerificationTargetKind.Window && origins[source(identity)] === old)
        if (old.selector == selector) return
        val next = Origin(selector, McpVerificationTargetKind.Phone)
        origins[source(identity)] = next
        packages.replaceAll { _, value -> if (value.origin === old) Bound(value.source, next) else value }
    }

    /** Origin kind of a bound package, or null when it has no verification route. */
    @Synchronized fun kind(info: PackageInfo): McpVerificationTargetKind? {
        live(); return packages[key(McpVerificationPackage.of(info))]?.origin?.kind
    }

    /** True for a phone-screen origin with no live route (never bound, or its
     * session retired); it needs a connected phone and an owner binding. */
    @Synchronized fun needsBinding(info: PackageInfo): Boolean {
        live(); val origin = packages[key(McpVerificationPackage.of(info))]?.origin ?: return false
        return origin.kind != McpVerificationTargetKind.Window && origin.selector == null
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
        // Coded, so the coordinator must check the caller's grant before reporting it.
        val selector = bound.origin.selector ?: throw McpRecaptureUnavailable(bound.origin.unbound)
        check(pending.size < 4) { "Verification recapture limit" }
        return Ticket(identity, selector, bound.origin.kind).also { pending[it] = bound }
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
            require(result.packageId != ticket.original.packageId && result.target == ticket.original.target)
            return McpVerificationRecapture(ticket.original, result, ticket.kind)
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
