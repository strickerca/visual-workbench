package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

class NativeCoreEventsTest {
    @Test fun theRealCoreReturnsAnOrderedLogThatCanBeReadIncrementally() {
        val all = readCoreEvents()
        assertTrue(all.events.zipWithNext().all { (before, after) -> before.sequence < after.sequence })
        val newest = all.events.lastOrNull()?.sequence ?: 0L
        assertTrue(readCoreEvents(newest).events.all { it.sequence > newest })
        assertTrue(all.reportLines().size == all.events.size)
    }

    @Test fun aProblemReportCarriesTheRealCoresEventsAndStillPassesItsPrivacyCheck() {
        val core = coreEventsForReport()
        assertNotNull(core)
        // prepare() refuses any report its own privacy audit does not accept.
        val report = PrivateDiagnosticLog().prepare(DiagnosticPlatform.Windows, DiagnosticMetrics(), core)
        assertTrue(report.jsonText.startsWith("{\"schema_version\":2,"))
        assertEquals(minOf(core.events.size, MAX_REPORTED_CORE_EVENTS), report.coreEventCount)
        assertTrue(report.jsonText.length <= MAX_DIAGNOSTIC_REPORT_CHARS)
    }
}
