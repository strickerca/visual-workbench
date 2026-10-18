package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.PackageInfo
import com.visualworkbench.shared.Rect
import com.visualworkbench.shared.SemanticDocument
import com.visualworkbench.shared.SemanticElement
import com.visualworkbench.shared.SemanticPlatform
import com.visualworkbench.shared.SemanticReference
import com.visualworkbench.shared.WorkflowBinding
import java.math.BigDecimal

/** Private projection of existing semantic records. Exported semantic.json has
 * no collection elapsed time, so it must not be turned into a fabricated full
 * SemanticDocument. No capture target model or new exported DTO is introduced. */
internal data class McpAlignmentSnapshot(
    val binding: WorkflowBinding, val snapshotId: String, val platform: SemanticPlatform,
    val elements: List<SemanticElement>,
) {
    companion object {
        fun of(value: SemanticDocument): McpAlignmentSnapshot {
            require(value.elements.size <= 4096)
            return McpAlignmentSnapshot(value.binding.copy(), value.snapshotId, value.platform,
                java.util.Collections.unmodifiableList(ArrayList(value.elements)))
        }

        /** Only supply bytes returned by the existing catalog.readFile with the
         * exact package ID/target/manifest hash and name semantic.json. Native
         * catalog verification supplies byte integrity; this decoder does not
         * authenticate arbitrary input. Missing snapshots return unresolved. */
        fun fromCheckedPackage(info: PackageInfo, semanticJson: ByteArray): McpAlignmentSnapshot? {
            require(semanticJson.size <= 16 * 1024 * 1024)
            val identity = McpVerificationPackage.of(info)
            val data = McpJson.decode(semanticJson)
            require(data["schema_version"] == BigDecimal.ONE && data["text_is_untrusted"] == true)
            require(data["project_id"] == identity.binding.projectId && data["document_id"] == identity.binding.documentId)
            val raw = data["elements"] as? List<*> ?: error("Semantic elements unavailable")
            require(raw.size <= 4096)
            val snapshot = data["snapshot_id"] as? String
            if (snapshot == null) { require(raw.isEmpty()); return null }
            require(snapshot.matches(Regex("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")))
            val platform = when (data["platform"]) {
                "uia" -> SemanticPlatform.Uia
                "android_ax" -> SemanticPlatform.AndroidAx
                "chromium_uia" -> SemanticPlatform.ChromiumUia
                else -> error("Semantic platform unavailable")
            }
            fun text(value: Any?, limit: Int, optional: Boolean = false): String? {
                if (value == null && optional) return null
                val result = value as? String ?: error("Semantic field unavailable")
                require(result.length <= limit && '\u0000' !in result)
                return result
            }
            val elements = raw.map { value ->
                val element = value as? Map<*, *> ?: error("Semantic element unavailable")
                val bounds = element["bounds_document"] as? List<*> ?: error("Semantic bounds unavailable")
                require(bounds.size == 4)
                val b = bounds.map { (it as? BigDecimal)?.toDouble() ?: error("Semantic coordinate unavailable") }
                // Native snapshots can retain invisible/fully clipped elements.
                // Preserve those records; only a usable alignment anchor must
                // have positive extent.
                require(b.all { it.isFinite() } && b[0] >= 0 && b[1] >= 0 && b[2] >= 0 && b[3] >= 0)
                val eid = requireNotNull(text(element["eid"], 300)); require(eid.startsWith("$snapshot/"))
                SemanticElement(eid, text(element["parent"], 300, true), requireNotNull(text(element["name"], 4096)),
                    requireNotNull(text(element["role"], 256)), text(element["automation_id"], 1024, true),
                    text(element["resource_id"], 1024, true), text(element["html_id"], 1024, true),
                    Rect(b[0], b[1], b[2], b[3]), element["bounds_clipped"] as? Boolean ?: error("Semantic clipping unavailable"),
                    requireNotNull(text(element["text"], 800)), element["enabled"] as? Boolean ?: error("Semantic state unavailable"),
                    element["focused"] as? Boolean ?: error("Semantic state unavailable"))
            }
            require(elements.map { it.eid }.toSet().size == elements.size)
            return McpAlignmentSnapshot(identity.binding, snapshot, platform, java.util.Collections.unmodifiableList(elements))
        }
    }
}

/** Explicit correspondence from two native semantic snapshots. Coordinates are
 * canonical document pixels, before package downscaling. This is a translation
 * observation, not an image-registration score or an owner pass/fail verdict. */
internal data class McpSemanticMarkerAlignment(
    val recapture: McpVerificationRecapture, val originalNumber: UInt,
    val originalSnapshotId: String, val recapturedSnapshotId: String,
    val originalEid: String, val recapturedEid: String,
    val beforeBounds: Rect, val afterBounds: Rect,
)

/** Uses existing semantic records; never matches names, text, array position or
 * raw snapshot-local EIDs. Missing, duplicated, clipped, resized or cross-platform
 * anchors remain unresolved. Image registration is a separate pending path. */
internal fun alignMcpMarker(
    recapture: McpVerificationRecapture, originalNumber: UInt, markerBounds: Rect,
    anchor: SemanticReference, before: McpAlignmentSnapshot, after: McpAlignmentSnapshot,
): McpSemanticMarkerAlignment? {
    require(before.binding == recapture.original.binding && after.binding == recapture.recaptured.binding)
    require(originalNumber >= 1u && originalNumber <= recapture.original.markerCount)
    require(before.elements.size <= 4096 && after.elements.size <= 4096)
    fun valid(rect: Rect) = listOf(rect.x, rect.y, rect.width, rect.height).all { it.isFinite() } &&
        rect.x >= 0 && rect.y >= 0 && rect.width > 0 && rect.height > 0 &&
        (rect.x + rect.width).isFinite() && (rect.y + rect.height).isFinite()
    require(valid(markerBounds))
    if (before.platform != after.platform || anchor.platform != before.platform) return null
    if (!anchor.eid.startsWith("${before.snapshotId}/") || !valid(anchor.boundsDocument)) return null
    val ids = listOf(anchor.automationId, anchor.resourceId, anchor.htmlId)
    if (ids.all { it.isNullOrBlank() }) return null
    if (ids.any { it != null && (it.length > 1024 || '\u0000' in it) }) return null
    fun sameIds(element: SemanticElement): Boolean =
        (anchor.automationId.isNullOrBlank() || element.automationId == anchor.automationId) &&
        (anchor.resourceId.isNullOrBlank() || element.resourceId == anchor.resourceId) &&
        (anchor.htmlId.isNullOrBlank() || element.htmlId == anchor.htmlId)
    val source = before.elements.singleOrNull(::sameIds) ?: return null
    val target = after.elements.singleOrNull(::sameIds) ?: return null
    if (source.eid != anchor.eid || source.boundsDocument != anchor.boundsDocument || source.role != anchor.role ||
        source.boundsClipped || target.boundsClipped || target.role != anchor.role ||
        !target.eid.startsWith("${after.snapshotId}/") || !valid(target.boundsDocument)) return null
    val a = source.boundsDocument; val b = target.boundsDocument
    // A translation-only match cannot silently stretch a resized element.
    if (a.width != b.width || a.height != b.height) return null
    val translated = markerBounds.copy(x = markerBounds.x + b.x - a.x, y = markerBounds.y + b.y - a.y)
    if (!valid(translated)) return null
    return McpSemanticMarkerAlignment(recapture, originalNumber, before.snapshotId, after.snapshotId,
        source.eid, target.eid, markerBounds.copy(), translated)
}
