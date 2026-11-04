package com.visualworkbench.desktop.mcp

import com.visualworkbench.desktop.desktopTunnelHierarchy
import com.visualworkbench.shared.*
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import java.nio.file.Path

/** Capture-capable phone-screen route (SEM-007 screencap + uiautomator through
 * one owner-opened tunnel session). Only the capture engine uses this form. */
internal interface McpPhoneCaptureRoute : McpPhoneScreenRoute {
    /** checkAvailable plus transient state (another tunnel operation or a
     * pending input confirmation). Call immediately before a recapture. */
    fun checkReady() = checkAvailable()
    /** One capture through the session's own operation admission. The capture
     * owner is closed after body returns or fails. Caller cancellation never
     * reaches the native session: its cancel hook is session-wide. */
    suspend fun <T> capture(body: suspend (WorkbenchTunnelCapture) -> T): T
}

/** The engine's selector dispatch. A phone route serves only verification
 * recapture; plain capture_window is refused for it. Null: a window selector. */
internal fun mcpSelectPhoneRoute(routes: Map<String, McpPhoneCaptureRoute>, selector: String,
    verification: Boolean): McpPhoneCaptureRoute? {
    val route = routes[selector] ?: return null
    if (!verification) throw McpRefused()
    if (route.retired) throw McpRecaptureUnavailable(McpRecaptureRefusal.PhoneDisconnected)
    return route
}

/** CAPTURE-008 on a Desktop window capture: the mirror guard refused. Report
 * it as such and never retry around it; other failures are unchanged. */
internal fun mcpGuardFailure(error: TunnelFailure): Exception =
    if (error.kind == TunnelFailureKind.FeedbackLoop) McpRecaptureUnavailable(McpRecaptureRefusal.FeedbackLoop) else error

/** Tunnel failures that describe source availability. Other failures stay as
 * ordinary refusals; nothing here bypasses the native feedback-loop guard. */
internal fun mcpRecaptureFailure(error: TunnelFailure): Exception = when (error.kind) {
    TunnelFailureKind.FeedbackLoop -> McpRecaptureUnavailable(McpRecaptureRefusal.FeedbackLoop)
    TunnelFailureKind.RuntimeUnavailable -> McpRecaptureUnavailable(McpRecaptureRefusal.TunnelRuntimeUnavailable)
    TunnelFailureKind.Closed, TunnelFailureKind.RetirementPending, TunnelFailureKind.Stale,
    TunnelFailureKind.AmbiguousDevice, TunnelFailureKind.UnauthorizedDevice ->
        McpRecaptureUnavailable(McpRecaptureRefusal.PhoneDisconnected)
    else -> error
}

/** Result of one phone recapture: the compiled package and its canonical project. */
internal class McpPhoneCompiled(val compiled: WorkbenchCompiledPackage, val project: Path)

/** Capture through the route, import into the engine's private work directory,
 * attach the uiautomator hierarchy as an Android accessibility snapshot in
 * capture pixels (same rules as the editor's tunnel capture) and compile with
 * the original's profile. A missing/stale/invalid hierarchy remains an honest
 * screenshot-only package, so alignment falls back to image registration.
 * createWork runs only after route admission and an actual screenshot, so a
 * refused recapture never leaves a private work directory behind. */
internal suspend fun compileMcpPhoneCapture(
    route: McpPhoneCaptureRoute, createWork: suspend () -> Path, core: WorkbenchCore, deviceId: String,
    profile: McpCaptureProfile, checkGrant: suspend () -> Unit,
    retain: (WorkbenchProject) -> Unit, settle: suspend () -> Unit,
    semanticUnavailable: suspend () -> Unit,
    semantics: (WorkbenchProject) -> WorkbenchSemantics = ::semanticWorkflows,
    compile: suspend (WorkbenchProject, PackageCompileOptions) -> WorkbenchCompiledPackage = { p, o -> p.compilePackage(o) },
): McpPhoneCompiled {
    route.checkReady(); checkGrant()
    return route.capture { captured ->
        checkGrant()
        val info = captured.info()
        val work = createWork()
        fun id() = core.newId(System.currentTimeMillis().toULong())
        val projectId = id(); val document = id(); val destination = work.resolve("project.vwb")
        // The native import owns its scratch inside work; the returned project
        // owns its durable files independently of the capture handle.
        val opened = captured.createProject(CreateFileProject(destination.toString(), projectId, document, id(), deviceId,
            "MCP phone capture", "", work.toString(), System.currentTimeMillis(),
            memoryBudgetBytes = 128uL * 1024uL * 1024uL)).also(retain)
        checkGrant()
        var snapshot: String? = null
        if (info.hierarchy == TunnelHierarchy.Present) {
            try {
                val observation = desktopTunnelHierarchy(info, captured.elements())
                val revision = opened.info()
                val ticket = semantics(opened).beginCapture(WorkflowBinding(revision.projectId, document, revision.hostSeq, revision.stateHash))
                try {
                    val described = ticket.describe()
                    require(described.sourceAssetId == info.assetId && described.captureSessionId == info.captureSessionId &&
                        described.frameId == info.frameId && described.geometryRevision.toULong() == info.sourceGeneration &&
                        described.width == info.width.toDouble() && described.height == info.height.toDouble())
                    val next = id()
                    val plan = ticket.prepare(WorkflowMetadata(id(), deviceId, revision.nextLamport, System.currentTimeMillis()), next,
                        SemanticPlatform.AndroidAx, observation.frameDeltaMs, observation.elapsedMs,
                        SemanticBoundsSpace.CapturePixels, observation.elements)
                    try { plan.commit(); snapshot = next } finally { withContext(NonCancellable) { plan.close() } }
                } finally { withContext(NonCancellable) { ticket.close() } }
            } catch (error: kotlinx.coroutines.CancellationException) { throw error }
            catch (_: Exception) { snapshot = null; semanticUnavailable() }
        } else semanticUnavailable()
        checkGrant()
        val current = opened.info()
        val result = compile(opened, PackageCompileOptions(WorkflowBinding(current.projectId, document, current.hostSeq, current.stateHash),
            id(), System.currentTimeMillis(), profile.target, snapshot, profile.includeWindowTitle))
        var owned: WorkbenchCompiledPackage? = result
        try {
            checkGrant(); currentCoroutineContext().ensureActive()
            // Keep the immutable canonical project for later source borrowing.
            settle()
            currentCoroutineContext().ensureActive()
            owned = null
            McpPhoneCompiled(result, destination)
        } finally { owned?.let { withContext(NonCancellable) { it.close() } } }
    }
}
