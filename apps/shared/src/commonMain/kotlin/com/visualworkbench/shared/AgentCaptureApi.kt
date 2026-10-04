package com.visualworkbench.shared

/** Display-only summary. Unknown never means capture is inactive. No token,
 * selector, external agent identity or permission crosses this interface. */
public data class AgentCaptureStatus(
    public val sequence: ULong = 0uL, public val connectionEpoch: ULong = 0uL,
    public val known: Boolean = false, public val activeGrantCount: UInt = 0u,
    public val activeCapture: Boolean = false, public val remainingMs: UInt = 0u,
)
public data class LocalAgentCaptureSummary(public val known: Boolean,
    public val activeGrantCount: UInt = 0u, public val activeCapture: Boolean = false,
    public val remainingMs: UInt = 0u)
public interface WorkbenchAgentCaptureStatus {
    public fun status(): AgentCaptureStatus
    public suspend fun waitStatus(afterSequence: ULong): AgentCaptureStatus
    /** Windows authoritative-host display source only. This does not grant or
     * revoke permission. The actual MCP owner must supply current truth. */
    public fun publish(value: LocalAgentCaptureSummary)
}
public expect fun agentCaptureStatus(link: ProjectLink): WorkbenchAgentCaptureStatus

/** Fixed trusted wording; no captured/agent text is rendered in this indicator. */
public fun agentCaptureStatusText(value: AgentCaptureStatus): String = when {
    !value.known -> "PC capture status unknown — reconnect to verify grants."
    value.activeCapture -> "PC agent capture is active · ${value.activeGrantCount} grant(s)"
    value.activeGrantCount != 0u -> "PC agent capture enabled · ${value.activeGrantCount} grant(s) · up to ${(value.remainingMs.toULong()+999uL)/1000uL}s"
    else -> "PC agent capture: no active grants"
}
