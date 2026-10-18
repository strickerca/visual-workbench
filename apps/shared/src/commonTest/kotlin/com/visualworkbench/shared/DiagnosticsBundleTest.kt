package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

class DiagnosticsBundleTest {
    @Test fun reportHasOnlyClosedSchemaStringsAndTypedMetrics() {
        val log = PrivateDiagnosticLog()
        DiagnosticEventCode.entries.forEachIndexed { i, code -> log.append(i.toLong(), code) }
        val report = log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics(
            startupMillis = 12, heapUsedBytes = 4096, traceSampleCount = 7,
            framesDecoded = 10, framesDropped = 2, echoRoundTripMillis = 3,
        ))
        val expected = """{"schema_version":1,"platform":"android","metrics":{"startup_ms":12,"heap_used_bytes":4096,"trace_samples":7,"frames_decoded":10,"frames_dropped":2,"echo_rtt_ms":3},"events_rotated":0,"events":[{"elapsed_ms":0,"code":"app_started"},{"elapsed_ms":1,"code":"trace_started"},{"elapsed_ms":2,"code":"trace_saved"},{"elapsed_ms":3,"code":"trace_interrupted"},{"elapsed_ms":4,"code":"trace_rejected"},{"elapsed_ms":5,"code":"report_prepared"},{"elapsed_ms":6,"code":"export_succeeded"},{"elapsed_ms":7,"code":"export_failed"},{"elapsed_ms":8,"code":"export_cancelled"}]}""" + "\n"
        assertEquals(expected, report.jsonText)
        assertTrue(diagnosticsPrivacyAudit(report.jsonText, report.eventCount, report.rotatedEventCount))
        assertEquals(9, report.eventCount)
    }

    @Test fun unavailableMetricsStayUnknownRatherThanBecomingZero() {
        val report = PrivateDiagnosticLog().prepare(DiagnosticPlatform.Windows, DiagnosticMetrics())
        assertTrue(report.jsonText.contains("\"platform\":\"windows\""))
        for (name in listOf("startup_ms", "heap_used_bytes", "trace_samples", "frames_decoded", "frames_dropped", "echo_rtt_ms")) {
            assertTrue(report.jsonText.contains("\"$name\":null"))
        }
        assertEquals(0, report.eventCount)
    }

    @Test fun rotationKeepsNewestEventsAndReportsDiscardedCount() {
        val log = PrivateDiagnosticLog(capacity = 2)
        log.append(10, DiagnosticEventCode.AppStarted)
        log.append(20, DiagnosticEventCode.TraceStarted)
        log.append(30, DiagnosticEventCode.TraceSaved)
        val report = log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics())
        assertEquals(2, report.eventCount)
        assertEquals(1L, report.rotatedEventCount)
        assertFalse(report.jsonText.contains("app_started"))
        assertTrue(report.jsonText.indexOf("trace_started") < report.jsonText.indexOf("trace_saved"))
    }

    @Test fun preparedPreviewCannotChangeWhenLaterEventsArrive() {
        val log = PrivateDiagnosticLog(capacity = 1)
        log.append(0, DiagnosticEventCode.AppStarted)
        val preview = log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics(traceSampleCount = 1))
        val original = preview.jsonText
        log.append(1, DiagnosticEventCode.ExportSucceeded)
        assertEquals(original, preview.jsonText)
        assertEquals(0L, preview.rotatedEventCount)
        assertFalse(preview.jsonText.contains("export_succeeded"))
        assertTrue(log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics()).jsonText.contains("export_succeeded"))
    }

    @Test fun invalidMetricsAndClockOrderAreRejectedBeforeExport() {
        assertFailsWith<IllegalArgumentException> { DiagnosticMetrics(heapUsedBytes = -1) }
        assertFailsWith<IllegalArgumentException> { DiagnosticMetrics(traceSampleCount = -1) }
        assertFailsWith<IllegalArgumentException> { DiagnosticMetrics(framesDecoded = Long.MAX_VALUE) }
        assertFailsWith<IllegalArgumentException> { PrivateDiagnosticLog(0) }
        assertFailsWith<IllegalArgumentException> { PrivateDiagnosticLog(257) }
        val log = PrivateDiagnosticLog()
        log.append(3, DiagnosticEventCode.AppStarted)
        assertFailsWith<IllegalArgumentException> { log.append(2, DiagnosticEventCode.ExportFailed) }
        assertFailsWith<IllegalArgumentException> { log.append(Long.MAX_VALUE, DiagnosticEventCode.ExportFailed) }
        assertEquals(1, log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics()).eventCount)
    }

    @Test fun maximumLogStillProducesBoundedAsciiPayload() {
        val log = PrivateDiagnosticLog(256)
        repeat(512) { log.append(it.toLong(), DiagnosticEventCode.TraceSaved) }
        val report = log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics(
            startupMillis = 9_007_199_254_740_991, heapUsedBytes = 9_007_199_254_740_991,
            traceSampleCount = Int.MAX_VALUE, framesDecoded = 9_007_199_254_740_991,
            framesDropped = 9_007_199_254_740_991, echoRoundTripMillis = 9_007_199_254_740_991,
        ))
        assertEquals(256, report.eventCount)
        assertEquals(256L, report.rotatedEventCount)
        assertTrue(report.jsonText.length <= 65_536)
        assertTrue(report.jsonText.all { it.code in 0..127 })
        assertTrue(report.jsonText.endsWith("\n"))
    }

    @Test fun rejectedClockCannotRotateOrChangeTheFrozenPreview() {
        val log = PrivateDiagnosticLog(capacity = 1)
        log.append(10L, DiagnosticEventCode.AppStarted)
        val before = log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics())
        assertFailsWith<IllegalArgumentException> { log.append(9L, DiagnosticEventCode.ExportSucceeded) }
        val after = log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics())
        assertEquals(before.jsonText, after.jsonText)
        assertEquals(0L, after.rotatedEventCount)
        assertFalse(after.jsonText.contains("export_succeeded"))
    }
    @Test fun androidCallerUsesOnlyHeapAndSampleCountsAndKeepsExactExportPreview() {
        // The actual DiagnosticsController.prepareReport inputs, with fake measurements.
        val log = PrivateDiagnosticLog().also { it.append(0L, DiagnosticEventCode.AppStarted) }
        log.append(10L, DiagnosticEventCode.TraceStarted)
        log.append(20L, DiagnosticEventCode.TraceSaved)
        log.append(30L, DiagnosticEventCode.ReportPrepared)
        val frozen = log.prepare(DiagnosticPlatform.Android, DiagnosticMetrics(heapUsedBytes = 4096L, traceSampleCount = 7))
        val expected = """{"schema_version":1,"platform":"android","metrics":{"startup_ms":null,"heap_used_bytes":4096,"trace_samples":7,"frames_decoded":null,"frames_dropped":null,"echo_rtt_ms":null},"events_rotated":0,"events":[{"elapsed_ms":0,"code":"app_started"},{"elapsed_ms":10,"code":"trace_started"},{"elapsed_ms":20,"code":"trace_saved"},{"elapsed_ms":30,"code":"report_prepared"}]}""" + "\n"
        assertEquals(expected, frozen.jsonText)
        assertTrue(diagnosticsPrivacyAudit(frozen.jsonText, 4, 0L))
        log.append(40L, DiagnosticEventCode.ExportSucceeded)
        // Both exporters consume this exact immutable string as UTF-8, not a new report.
        assertTrue(expected.encodeToByteArray().contentEquals(frozen.jsonText.encodeToByteArray()))
        assertFalse(frozen.jsonText.contains("export_succeeded"))
    }

    @Test fun desktopCallerKeepsUnknownFrameCountsAndEchoDistinctFromDrawingLatency() {
        // Main's actual diagnostic preparation path uses startup, heap and measured echo only.
        val log = PrivateDiagnosticLog().also { it.append(0L, DiagnosticEventCode.AppStarted) }
        log.append(47L, DiagnosticEventCode.ReportPrepared)
        val frozen = log.prepare(DiagnosticPlatform.Windows, DiagnosticMetrics(startupMillis = 47L,
            heapUsedBytes = 8192L, echoRoundTripMillis = 3L))
        val expected = """{"schema_version":1,"platform":"windows","metrics":{"startup_ms":47,"heap_used_bytes":8192,"trace_samples":null,"frames_decoded":null,"frames_dropped":null,"echo_rtt_ms":3},"events_rotated":0,"events":[{"elapsed_ms":0,"code":"app_started"},{"elapsed_ms":47,"code":"report_prepared"}]}""" + "\n"
        assertEquals(expected, frozen.jsonText)
        assertTrue(diagnosticsPrivacyAudit(frozen.jsonText, 2, 0L))
        log.append(48L, DiagnosticEventCode.ExportCancelled)
        assertEquals(expected, frozen.jsonText)
    }

    @Test fun failedAuditAndRejectedPreparationLeaveSourceLogAndPriorPreviewIntact() {
        val log = PrivateDiagnosticLog(capacity = 2)
        log.append(1L, DiagnosticEventCode.AppStarted)
        log.append(2L, DiagnosticEventCode.ReportPrepared)
        log.append(3L, DiagnosticEventCode.ExportCancelled)
        val before = log.prepare(DiagnosticPlatform.Windows, DiagnosticMetrics(heapUsedBytes = 16L))
        val malformed = before.jsonText.replace("\"heap_used_bytes\":16", "\"heap_used_bytes\":\"fake-test-credential\"")
        repeat(32) { assertFalse(diagnosticsPrivacyAudit(malformed, before.eventCount, before.rotatedEventCount)) }
        assertFailsWith<IllegalArgumentException> {
            PreparedDiagnosticsReport.create(DiagnosticPlatform.Windows, DiagnosticMetrics(), listOf(
                PrivateDiagnosticEvent(2L, DiagnosticEventCode.AppStarted),
                PrivateDiagnosticEvent(1L, DiagnosticEventCode.ReportPrepared)), 0L)
        }
        val after = log.prepare(DiagnosticPlatform.Windows, DiagnosticMetrics(heapUsedBytes = 16L))
        assertEquals(before.jsonText, after.jsonText)
        assertEquals(before.eventCount, after.eventCount)
        assertEquals(before.rotatedEventCount, after.rotatedEventCount)
        assertTrue(diagnosticsPrivacyAudit(before.jsonText, before.eventCount, before.rotatedEventCount))
    }
}
