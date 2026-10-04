package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.LocalAgentCaptureSummary

/** The only projection permitted onto the peer wire. Grant selectors/tokens,
 * agent names/IDs and private project text never enter the summary. */
internal fun mcpGrantTelemetry(lifetime: McpLifetimeState, nowNs: Long): LocalAgentCaptureSummary {
    val owner = lifetime.owner
    if (owner == null) return LocalAgentCaptureSummary(known = !lifetime.starting && !lifetime.closed && lifetime.failureStage == null)
    if (lifetime.closed && lifetime.issue != null) return LocalAgentCaptureSummary(false)
    return mcpGrantTelemetry(owner.state.value, nowNs)
}
internal fun mcpGrantTelemetry(state: McpDesktopState, nowNs: Long): LocalAgentCaptureSummary {
    if (state.grants.size > 64) return LocalAgentCaptureSummary(false)
    val live = state.grants.filter { it.expiresNs > nowNs }
    if (live.size > 64) return LocalAgentCaptureSummary(false)
    val remaining = try { live.maxOfOrNull { Math.subtractExact(it.expiresNs, nowNs).coerceAtLeast(0) } ?: 0L }
        catch (_: ArithmeticException) { return LocalAgentCaptureSummary(false) }
    val remainingMs = remaining / 1_000_000L + if (remaining % 1_000_000L == 0L) 0 else 1
    if (remainingMs > 600_000L) return LocalAgentCaptureSummary(false)
    return LocalAgentCaptureSummary(true, live.size.toUInt(), state.activeCapture != null, remainingMs.toUInt())
}

/** Whole lifetime, including permission without an in-flight capture and a
 * revoked capture still joining its native producer. */
internal fun mcpGrantIndicatorVisible(state: McpDesktopState): Boolean =
    state.grants.isNotEmpty() || state.activeCapture != null
