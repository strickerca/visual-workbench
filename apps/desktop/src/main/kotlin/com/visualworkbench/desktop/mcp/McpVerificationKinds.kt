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
}
internal class McpRecaptureUnavailable(val reason: McpRecaptureRefusal) : Exception(reason.message)

/** Availability/identity view of a PC-side phone-screen source: exactly one
 * owner-opened adb tunnel session. It never opens, confirms or widens that
 * session, is never persisted and refuses once the session retires. The
 * capture-capable form is McpPhoneCaptureRoute; the coordinator needs only this. */
internal interface McpPhoneScreenRoute {
    /** Display text for the owner's grant list; not an identity proof. */
    val description: String
    /** Fail closed: runtime admission and the exact live session. */
    fun checkAvailable()
}

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
