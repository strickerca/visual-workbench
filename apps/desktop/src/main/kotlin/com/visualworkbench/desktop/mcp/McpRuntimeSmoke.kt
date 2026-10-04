package com.visualworkbench.desktop.mcp

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.nio.file.Path

/** Explicit packaged diagnostic only. Uses the ordinary service factory and
 * lifecycle without publishing packages, granting capture or sending messages.
 * The application still owns shutdown if any check or cancellation fails. */
internal suspend fun verifyPackagedMcpLifecycle(service: DesktopMcpLifetime, anchor: Path) {
    withContext(Dispatchers.IO) {
        val duplicate = DesktopAppOwner.claim(anchor)
        try { check(duplicate == null) { "Duplicate application owner admitted" } }
        finally { duplicate?.close() }
    }
    service.start()
    val first = service.state.value.owner ?: throw McpRefused(service.state.value.failureStage ?: McpFailureStage.unknown)
    requireIdleMcp(first)
    service.start()
    check(service.state.value.owner === first)
    service.stop()
    check(first.state.value.closed && service.state.value.owner == null)
    service.start()
    val second = service.state.value.owner ?: throw McpRefused(service.state.value.failureStage ?: McpFailureStage.unknown)
    check(second !== first && second.state.value.port == first.state.value.port)
    requireIdleMcp(second)
    service.stop()
    check(second.state.value.closed && service.state.value.owner == null && !service.state.value.closed)
}

private fun requireIdleMcp(owner: DesktopMcpCoordinator) {
    val state = owner.state.value
    check(!state.closed && state.port in 1..65535 && state.agents.isEmpty() && state.grants.isEmpty() &&
        state.targets.isEmpty() && state.activeCapture == null && state.sendPreview == null)
}
