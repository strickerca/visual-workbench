package com.visualworkbench.desktop.mcp

import org.junit.Assert.*
import org.junit.Test

class McpGrantTelemetryTest {
    @Test fun permissionWithoutAnActualCaptureKeepsTheDesktopIndicatorVisible() {
        val grant = McpCaptureGrant("private-agent", listOf("private-window"), 30_000_000_000L)
        val state = McpDesktopState(0, grants = listOf(grant))
        assertTrue(mcpGrantIndicatorVisible(state))
        val wire = mcpGrantTelemetry(state, 0)
        assertTrue(wire.known); assertEquals(1u, wire.activeGrantCount); assertFalse(wire.activeCapture)
        assertEquals(30_000u, wire.remainingMs)
        assertFalse(wire.toString().contains("private"))
    }
    @Test fun revokeAndExpiryReplaceTheWholeCountButActualCaptureRemainsVisibleUntilSettled() {
        val state = McpDesktopState(0, grants = listOf(McpCaptureGrant("a", listOf("w"), 9),
            McpCaptureGrant("b", listOf("x"), 1_000_010)), activeCapture = McpCaptureActivity("capture", "a", "w"))
        val atExpiry = mcpGrantTelemetry(state, 10)
        assertEquals(1u, atExpiry.activeGrantCount); assertEquals(1u, atExpiry.remainingMs); assertTrue(atExpiry.activeCapture)
        val revoked = state.copy(grants = emptyList())
        assertTrue(mcpGrantIndicatorVisible(revoked))
        assertEquals(0u, mcpGrantTelemetry(revoked, 10).activeGrantCount)
        assertTrue(mcpGrantTelemetry(revoked, 10).activeCapture)
        val finished = revoked.copy(activeCapture = null)
        assertFalse(mcpGrantIndicatorVisible(finished)); assertFalse(mcpGrantTelemetry(finished, 10).activeCapture)
    }
    @Test fun unavailableStartingOrUncertainOwnerNeverMeansInactive() {
        assertTrue(mcpGrantTelemetry(McpLifetimeState(), 0).known)
        assertFalse(mcpGrantTelemetry(McpLifetimeState(starting = true), 0).known)
        assertFalse(mcpGrantTelemetry(McpLifetimeState(closed = true), 0).known)
        assertFalse(mcpGrantTelemetry(McpLifetimeState(failureStage = McpFailureStage.unknown), 0).known)
    }
    @Test fun malformedCountLifetimeOrSubtractionRefusesRatherThanTruncates() {
        fun state(expiry: Long, count: Int = 1) = McpDesktopState(0,
            grants = List(count) { McpCaptureGrant("$it", listOf("w"), expiry) })
        assertFalse(mcpGrantTelemetry(state(1, 65), 0).known)
        assertFalse(mcpGrantTelemetry(state(600_000_000_001L), 0).known)
        assertFalse(mcpGrantTelemetry(state(Long.MAX_VALUE), Long.MIN_VALUE).known)
        assertEquals(600_000u, mcpGrantTelemetry(state(600_000_000_000L), 0).remainingMs)
    }
}
