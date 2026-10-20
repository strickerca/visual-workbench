package com.visualworkbench.desktop.mcp

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Test

/** Synthetic classifiers only; no packaged runtime, native service or listener. */
class McpRuntimeSmokeTest {
    @Test fun unfinishedStepReportsMeasuredSubphaseWithoutChangingFailureMeaning() = runBlocking {
        var now = 0L
        val startup = McpStartupProgress { now }
        startup.begin(); startup.inventory(1514, 119_000_000); startup.phase(McpFailureStage.bundle_verify)
        startup.verifiedFile(1024)
        val lines = mutableListOf<String>()
        val progress = McpSmokeProgress({ now }, lines::add)
        progress.observe(startup)
        val failure = IllegalStateException("synthetic private failure")
        try {
            progress.at(McpSmokeStep.second_start) { now = 12_000_000; throw failure }
        } catch (error: IllegalStateException) { assertSame(failure, error) }
        assertEquals(1, lines.size)
        assertTrue(lines.single().startsWith("VW_MCP_TIMING step=second_start completed=false step_ms=12 attempt=1 phase=bundle_verify start_ms=12 "))
        assertTrue(lines.single().contains("hashed_files=1 hashed_bytes=1024"))
        assertFalse(lines.single().contains("synthetic private"))
        assertEquals("VW_DESKTOP_MCP failed=true stage=unknown step=second_start reason=invariant", progress.failureMarker(failure))
    }

    @Test fun unavailableDiagnosticSinkCannotChangeTheOperationResult() = runBlocking {
        val progress = McpSmokeProgress({ 0L }) { throw IllegalArgumentException("synthetic sink unavailable") }
        assertEquals(42, progress.at(McpSmokeStep.first_idle) { 42 })
        val original = IllegalStateException("synthetic operation refused")
        try { progress.at(McpSmokeStep.second_idle) { throw original } }
        catch (error: IllegalStateException) { assertSame(original, error) }
    }

    @Test fun timeoutRetainsTheUnfinishedStepWithoutBecomingAnInvariantFailure() = runBlocking {
        val progress = McpSmokeProgress()
        val marker = try {
            withTimeout(10) { progress.at(McpSmokeStep.second_start) { awaitCancellation() } }
        } catch (error: TimeoutCancellationException) {
            progress.failureMarker(error)
        }
        assertEquals("VW_DESKTOP_MCP failed=true stage=unknown step=second_start reason=timeout", marker)
    }

    @Test fun failedInvariantKeepsItsStepAndRunsTheOriginalCleanup() = runBlocking {
        val progress = McpSmokeProgress()
        var cleaned = false
        val marker = try {
            progress.at(McpSmokeStep.first_stop) {
                try { check(false) { "synthetic private failure text" } }
                finally { cleaned = true }
            }
        } catch (error: IllegalStateException) {
            progress.failureMarker(error)
        }
        assertTrue(cleaned)
        assertEquals("VW_DESKTOP_MCP failed=true stage=unknown step=first_stop reason=invariant", marker)
    }

    @Test fun refusalPreservesEveryFixedStageWithoutCopyingExceptionText() = runBlocking {
        val progress = McpSmokeProgress()
        progress.at(McpSmokeStep.first_start) { }
        for (stage in McpFailureStage.entries) {
            assertEquals("VW_DESKTOP_MCP failed=true stage=${stage.name} step=first_start reason=refused",
                progress.failureMarker(McpRefused(stage)))
        }
    }

    @Test fun cancellationAndUnexpectedFailuresUseOnlyFixedCodes() {
        val progress = McpSmokeProgress()
        assertEquals("VW_DESKTOP_MCP failed=true stage=unknown step=duplicate_owner reason=cancelled",
            progress.failureMarker(CancellationException("synthetic private cancellation text")))
        assertEquals("VW_DESKTOP_MCP failed=true stage=unknown step=duplicate_owner reason=unexpected",
            progress.failureMarker(Exception("synthetic private runtime text")))
        assertEquals("VW_DESKTOP_MCP failed=true stage=unknown step=duplicate_owner reason=invariant",
            progress.failureMarker(IllegalArgumentException("synthetic private invariant text")))
    }
}
