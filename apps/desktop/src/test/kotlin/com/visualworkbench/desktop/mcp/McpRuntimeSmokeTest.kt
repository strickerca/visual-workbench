package com.visualworkbench.desktop.mcp

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Synthetic classifiers only; no packaged runtime, native service or listener. */
class McpRuntimeSmokeTest {
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
