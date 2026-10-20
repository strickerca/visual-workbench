package com.visualworkbench.desktop.mcp

import org.junit.Assert.*
import org.junit.Test

class McpStartupProgressTest {
    @Test fun monotonicPhaseDurationsAndHashCountersFreezeWhenCreatorReturns() {
        var now = 0L
        val progress = McpStartupProgress { now }
        progress.begin()
        now = 2_000_000; progress.phase(McpFailureStage.bundle_manifest)
        progress.inventory(2, 9)
        now = 5_000_000; progress.phase(McpFailureStage.bundle_verify)
        progress.verifiedFile(4); progress.verifiedFile(5)
        now = 9_000_000; progress.finish()
        now = 99_000_000
        val value = progress.snapshot()
        assertEquals(1, value.attempt); assertEquals(9L, value.elapsedMillis)
        assertEquals(2L, value.phaseMillis[McpStartupPhase.unknown.ordinal])
        assertEquals(3L, value.phaseMillis[McpStartupPhase.bundle_manifest.ordinal])
        assertEquals(4L, value.phaseMillis[McpStartupPhase.bundle_verify.ordinal])
        assertEquals(2, value.verifiedFiles); assertEquals(9L, value.verifiedBytes)
    }

    @Test fun anotherActualProducerResetsPerCycleWorkAndAdvancesAttempt() {
        var now = 0L
        val progress = McpStartupProgress { now }
        progress.begin(); progress.inventory(1, 8); progress.verifiedFile(8)
        now = 10_000_000; progress.finish()
        now = 20_000_000; progress.begin()
        val value = progress.snapshot()
        assertEquals(2, value.attempt); assertEquals(McpStartupPhase.unknown, value.phase)
        assertEquals(0L, value.elapsedMillis); assertTrue(value.phaseMillis.all { it == 0L })
        assertEquals(0, value.files); assertEquals(0L, value.bytes)
        assertEquals(0, value.verifiedFiles); assertEquals(0L, value.verifiedBytes)
    }

    @Test fun countersAndTimeCannotProduceUnboundedOrNegativeDiagnostics() {
        var now = 10L
        val progress = McpStartupProgress { now }
        progress.begin(); progress.inventory(Int.MAX_VALUE, Long.MAX_VALUE)
        progress.verifiedFile(Long.MAX_VALUE)
        now = Long.MAX_VALUE
        val value = progress.snapshot()
        assertEquals(3_600_000L, value.elapsedMillis)
        assertEquals(20_000, value.files); assertEquals(1_073_741_824L, value.bytes)
        assertEquals(1, value.verifiedFiles); assertEquals(value.bytes, value.verifiedBytes)
        assertTrue(value.phaseMillis.all { it in 0L..3_600_000L })
        now = 0
        assertEquals(0L, progress.snapshot().elapsedMillis)
        assertTrue(progress.snapshot().phaseMillis.all { it == 0L })
    }

    @Test fun fixedFieldsHaveNoPathIdentityOrArbitraryPayloadChannel() {
        val progress = McpStartupProgress { 0L }
        progress.begin(); progress.phase(McpFailureStage.bundle_inventory); progress.inventory(1, 4)
        val fields = progress.snapshot().fields().split(' ')
        val expected = setOf("attempt", "phase", "start_ms", "files", "bytes", "hashed_files", "hashed_bytes") +
            McpStartupPhase.entries.map { "${it.name}_ms" }
        assertEquals(expected, fields.map { it.substringBefore('=') }.toSet())
        assertEquals(expected.size, fields.size)
        assertTrue(fields.all { it.matches(Regex("[a-z_]+=(?:[0-9]+|bundle_inventory)")) })
        assertFalse(progress.snapshot().fields().contains("VW_DESKTOP_"))
    }
}
