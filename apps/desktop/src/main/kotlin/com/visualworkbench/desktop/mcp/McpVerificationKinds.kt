package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.WorkflowBinding

/** What produced the original that a verification route recaptures.
 * Window: an owner-selected Desktop window (WGC/UIA). Tunnel: a PC-side phone
 * capture through one owner-opened adb tunnel session (screencap+uiautomator).
 * Phone: a phone-app screen capture that reached this PC inside a project from
 * the session link; it has no PC route until the owner binds a live tunnel. */
internal enum class McpVerificationTargetKind(val wire: String) { Window("window"), Tunnel("tunnel"), Phone("phone") }

/** Fixed, nonsecret refusal categories. They name why a source is unavailable;
 * they never carry a selector, device identity, path or native message. */
internal enum class McpRecaptureRefusal(val code: String, val message: String) {
    SourceUnavailable("recapture_source_unavailable", "The original capture source is unavailable for recapture."),
    TunnelRuntimeUnavailable("tunnel_runtime_unavailable", "Phone recapture needs the Android tunnel runtime, which is not admitted in this package."),
    PhoneDisconnected("phone_disconnected", "The phone's USB tunnel session is closed or was replaced. Reconnect it and bind again."),
    PhoneRouteUnbound("phone_route_unbound", "This phone capture came from the phone. Bind the connected USB phone before recapturing it."),
    FeedbackLoop("feedback_loop", "Recapture refused: the source shows the phone mirror window."),
    SourceBusy("source_busy", "The phone tunnel is busy or waiting for a confirmation. Retry after it settles."),
    PhoneMismatch("phone_mismatch", "The connected phone's screen size does not match this phone capture. Bind the phone it came from."),
}
internal class McpRecaptureUnavailable(val reason: McpRecaptureRefusal) : Exception(reason.message)

/** Availability/identity view of a PC-side phone-screen source: exactly one
 * owner-opened adb tunnel session. It never opens, confirms or widens that
 * session, is never persisted and refuses once the session retires. The
 * capture-capable form is McpPhoneCaptureRoute; the coordinator needs only this. */
internal interface McpPhoneScreenRoute {
    /** Display text naming the connected device model (never a serial); not an
     * identity proof. */
    val description: String
    /** True once the exact session closed or was replaced; never live again. */
    val retired: Boolean
    /** Fail closed: runtime admission and the exact live session. It does not
     * consider transient busy state, so a capture can register during delivery. */
    fun checkAvailable()
}

/** Durable, owner-visible label written as the note of the owner's own
 * recapture comparison receipt, so an owner-attested pairing stays labeled when
 * a saved review is reopened. Window recaptures keep the empty note. */
internal fun mcpRecaptureNote(kind: McpVerificationTargetKind): String = when (kind) {
    McpVerificationTargetKind.Window -> ""
    McpVerificationTargetKind.Tunnel -> "Recapture source: phone screen through the capturing USB tunnel session."
    McpVerificationTargetKind.Phone -> "Recapture source: phone screen through an owner-bound USB tunnel (owner-attested pairing)."
}
/** True when [note] could pass for an owner-written recapture label: after
 * dropping invisible format code points (supplementary ones included) and
 * folding every Unicode space or separator to one space, it starts with the
 * labels' shared prefix (any case). Agent returns refuse these notes, so a
 * reopened review's kind cannot be forged and Compare text cannot pass for a
 * label by spacing tricks. Look-alike letters from other scripts can still
 * pass a text check; Compare shows agent notes as untrusted content. */
internal fun mcpReservedRecaptureNote(note: String): Boolean {
    val visible = buildString {
        note.codePoints().forEach { point -> if (Character.getType(point) != Character.FORMAT.toInt()) appendCodePoint(point) }
    }.replace(Regex("[\\s\\p{Z}]+"), " ").trim()
    return visible.startsWith(RECAPTURE_LABEL_PREFIX, ignoreCase = true)
}
private const val RECAPTURE_LABEL_PREFIX = "Recapture source:"
internal fun mcpRecaptureKind(note: String): McpVerificationTargetKind =
    McpVerificationTargetKind.entries.firstOrNull { it != McpVerificationTargetKind.Window && mcpRecaptureNote(it) == note }
        ?: McpVerificationTargetKind.Window

/** Minted only by the tunnel owner from the exact capture it is delivering to
 * the editor and the live route that produced it. No agent or portable input. */
internal class McpPhoneCaptureOrigin internal constructor(
    val binding: WorkflowBinding, val sourceAssetId: String, internal val route: McpPhoneScreenRoute,
)

/** Reads only the checked manifest's source.capture.platform. Missing/invalid -> null. */
internal fun mcpManifestCapturePlatform(manifest: ByteArray): String? = try {
    val header = checkedJsonArrayParts(manifest, "markers", 512).header
    ((header["source"] as? Map<*, *>)?.get("capture") as? Map<*, *>)?.get("platform") as? String
} catch (_: IllegalArgumentException) { null } catch (_: IllegalStateException) { null }
    catch (_: java.nio.charset.CharacterCodingException) { null }
