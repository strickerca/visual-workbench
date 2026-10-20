package com.visualworkbench.desktop.mcp

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.withContext
import java.nio.file.Path

/** Fixed diagnostic vocabulary only. Never retain exception text or runtime paths. */
internal enum class McpSmokeStep {
    duplicate_owner, first_start, first_idle, repeated_start, first_stop,
    second_start, second_identity, second_idle, second_stop,
}
internal enum class McpSmokeFailureReason { timeout, invariant, refused, cancelled, unexpected }
internal class McpSmokeProgress(private val nowNanos:()->Long=System::nanoTime,
    private val diagnostic:(String)->Unit={println(it)}) {
    private var current = McpSmokeStep.duplicate_owner
    private var startup:McpStartupProgress?=null
    fun observe(value:McpStartupProgress){startup=value}
    suspend fun <T> at(step: McpSmokeStep, body: suspend () -> T): T {
        current = step
        val started=nowNanos();var completed=false
        try{return body().also{completed=true}}
        finally{
            val elapsed=McpStartupProgress.elapsedMillis(started,nowNanos())
            val details=startup?.snapshot()?.fields()?:"attempt=0 phase=unknown start_ms=0"
            // Supplementary fixed counters do not alter the strict success/failure
            // marker, deadline, producer ownership or cleanup predicates.
            try{diagnostic("VW_MCP_TIMING step=${step.name} completed=$completed step_ms=$elapsed $details")}
            catch(_:Exception){/* A diagnostic sink cannot change the operation. */}
        }
    }
    fun failureMarker(error: Exception): String {
        val stage = (error as? McpRefused)?.stage ?: McpFailureStage.unknown
        val reason = when (error) {
            is TimeoutCancellationException -> McpSmokeFailureReason.timeout
            is CancellationException -> McpSmokeFailureReason.cancelled
            is McpRefused -> McpSmokeFailureReason.refused
            is IllegalStateException, is IllegalArgumentException -> McpSmokeFailureReason.invariant
            else -> McpSmokeFailureReason.unexpected
        }
        return "VW_DESKTOP_MCP failed=true stage=${stage.name} step=${current.name} reason=${reason.name}"
    }
}

/** Explicit packaged diagnostic only. Uses the ordinary service factory and
 * lifecycle without publishing packages, granting capture or sending messages.
 * The application still owns shutdown if any check or cancellation fails. */
internal suspend fun verifyPackagedMcpLifecycle(service: DesktopMcpLifetime, anchor: Path, progress: McpSmokeProgress) {
    progress.observe(service.startupProgress)
    progress.at(McpSmokeStep.duplicate_owner) {
        withContext(Dispatchers.IO) {
            val duplicate = DesktopAppOwner.claim(anchor)
            try { check(duplicate == null) { "Duplicate application owner admitted" } }
            finally { duplicate?.close() }
        }
    }
    val first = progress.at(McpSmokeStep.first_start) {
        service.start()
        service.state.value.owner ?: throw McpRefused(service.state.value.failureStage ?: McpFailureStage.unknown)
    }
    progress.at(McpSmokeStep.first_idle) { requireIdleMcp(first) }
    progress.at(McpSmokeStep.repeated_start) {
        service.start()
        check(service.state.value.owner === first)
    }
    progress.at(McpSmokeStep.first_stop) {
        service.stop()
        check(first.state.value.closed && service.state.value.owner == null)
    }
    val second = progress.at(McpSmokeStep.second_start) {
        service.start()
        service.state.value.owner ?: throw McpRefused(service.state.value.failureStage ?: McpFailureStage.unknown)
    }
    progress.at(McpSmokeStep.second_identity) {
        check(second !== first && second.state.value.port == first.state.value.port)
    }
    progress.at(McpSmokeStep.second_idle) { requireIdleMcp(second) }
    progress.at(McpSmokeStep.second_stop) {
        service.stop()
        check(second.state.value.closed && service.state.value.owner == null && !service.state.value.closed)
    }
}

private fun requireIdleMcp(owner: DesktopMcpCoordinator) {
    val state = owner.state.value
    check(!state.closed && state.port in 1..65535 && state.agents.isEmpty() && state.grants.isEmpty() &&
        state.targets.isEmpty() && state.activeCapture == null && state.sendPreview == null)
}
