package com.visualworkbench.desktop.diagnostics

import com.visualworkbench.shared.DiagnosticEventCode
import com.visualworkbench.shared.DiagnosticMetrics
import com.visualworkbench.shared.DiagnosticPlatform
import com.visualworkbench.shared.PrivateDiagnosticLog
import java.nio.file.Files
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class DiagnosticsPublicationTest {
    @get:Rule val temporary = TemporaryFolder()

    private fun prepared() = PrivateDiagnosticLog().apply {
        append(0L, DiagnosticEventCode.AppStarted)
        append(12L, DiagnosticEventCode.ReportPrepared)
    }.prepare(DiagnosticPlatform.Windows, DiagnosticMetrics(startupMillis = 11L))

    @Test fun savesFrozenPreviewBytesWithoutRecollectingOrChangingNewline() {
        val log = PrivateDiagnosticLog().apply { append(0L, DiagnosticEventCode.AppStarted) }
        val report = log.prepare(DiagnosticPlatform.Windows, DiagnosticMetrics(heapUsedBytes = 10L))
        log.append(1L, DiagnosticEventCode.ExportFailed)
        val directory = temporary.newFolder().toPath()
        val destination = directory.resolve("report.json")
        val writer = DiagnosticPublication(report, destination)
        assertEquals(DiagnosticWriteResult.Saved, writer.write())
        assertArrayEquals(report.jsonText.toByteArray(Charsets.UTF_8), Files.readAllBytes(destination))
        assertEquals('\n', Files.readString(destination).last())
        assertFalse(Files.readString(destination).contains("export_failed"))
        assertTrue(writer.published)
        assertFalse(writer.cleanupPending)
        Files.list(directory).use { assertEquals(1L, it.count()) }
    }

    @Test fun existingDestinationRemainsUnchangedAndNoSiblingIsCreated() {
        val directory = temporary.newFolder().toPath()
        val destination = directory.resolve("report.json")
        val original = byteArrayOf(1, 2, 3, 4)
        Files.write(destination, original)
        val writer = DiagnosticPublication(prepared(), destination)
        assertEquals(DiagnosticWriteResult.Failed, writer.write())
        assertArrayEquals(original, Files.readAllBytes(destination))
        assertFalse(writer.published)
        assertFalse(writer.cleanupPending)
        Files.list(directory).use { assertEquals(1L, it.count()) }
    }

    @Test fun destinationCreatedAfterAdmissionWinsWithoutBeingReplaced() {
        val directory = temporary.newFolder().toPath()
        val destination = directory.resolve("report.json")
        val original = byteArrayOf(5, 6, 7, 8)
        var competingFileCreated = false
        val writer = DiagnosticPublication(prepared(), destination, nowNanos = {
            val siblingHasBytes = Files.list(directory).use { paths ->
                paths.anyMatch { it.fileName.toString().endsWith(".tmp") && Files.size(it) > 0 }
            }
            if (!competingFileCreated && siblingHasBytes) {
                Files.write(destination, original, java.nio.file.StandardOpenOption.CREATE_NEW)
                competingFileCreated = true
            }
            0L
        })
        assertEquals(DiagnosticWriteResult.Failed, writer.write())
        assertTrue(competingFileCreated)
        assertArrayEquals(original, Files.readAllBytes(destination))
        assertFalse(writer.published)
        assertFalse(writer.cleanupPending)
        Files.list(directory).use { assertEquals(1L, it.count()) }
    }

    @Test fun cancellationBeforeWorkCreatesNoFiles() {
        val directory = temporary.newFolder().toPath()
        val gate = DiagnosticPublicationGate()
        assertTrue(gate.cancel())
        val writer = DiagnosticPublication(prepared(), directory.resolve("report.json"), gate)
        assertEquals(DiagnosticWriteResult.Cancelled, writer.write())
        assertFalse(writer.published)
        assertFalse(writer.cleanupPending)
        Files.list(directory).use { assertEquals(0L, it.count()) }
    }

    @Test fun cancellationDuringPreparationRetiresItsOwnedSiblingWithoutPublishing() {
        val directory = temporary.newFolder().toPath()
        val gate = DiagnosticPublicationGate()
        var calls = 0
        val writer = DiagnosticPublication(prepared(), directory.resolve("report.json"), gate, nowNanos = {
            // The third clock read occurs at admission before CREATE_NEW. The
            // next admission sees cancellation while the sibling is owned.
            if (++calls == 3) gate.cancel()
            0L
        })
        assertEquals(DiagnosticWriteResult.Cancelled, writer.write())
        assertFalse(writer.published)
        assertFalse(writer.cleanupPending)
        Files.list(directory).use { assertEquals(0L, it.count()) }
    }

    @Test fun elapsedDeadlineCannotPublishAReport() {
        val directory = temporary.newFolder().toPath()
        var calls = 0
        val writer = DiagnosticPublication(prepared(), directory.resolve("report.json"), nowNanos = {
            if (++calls == 1) 0L else 6_000_000_000L
        })
        assertEquals(DiagnosticWriteResult.Failed, writer.write())
        assertFalse(writer.published)
        assertFalse(writer.cleanupPending)
        Files.list(directory).use { assertEquals(0L, it.count()) }
    }

    @Test fun cancellationAndCommitHaveDistinctOutcomes() {
        val cancelled = DiagnosticPublicationGate()
        assertTrue(cancelled.cancel())
        assertFalse(cancelled.beginCommit())
        val committing = DiagnosticPublicationGate()
        assertTrue(committing.beginCommit())
        assertFalse(committing.cancel()) // A completed save cannot be reported as no-write.
        assertFalse(committing.isCancelled())
        committing.finishCommit()
        assertFalse(committing.cancel())
    }

    @Test fun unavailableOrInvalidEchoIsNullAndFiniteMeasuredEchoRoundsSafely() {
        for (value in listOf(null, Double.NaN, Double.POSITIVE_INFINITY, -1.0, 9_007_199_254_740_992.0)) {
            assertNull(measuredEchoRoundTripMillis(1u, value))
        }
        assertNull(measuredEchoRoundTripMillis(0u, 10.0))
        assertEquals(11L, measuredEchoRoundTripMillis(1u, 10.6))
        assertEquals(9_007_199_254_740_991L, measuredEchoRoundTripMillis(1u, 9_007_199_254_740_991.0))
        assertNull(measuredDiagnosticValue(-1L))
        assertNull(measuredDiagnosticValue(9_007_199_254_740_992L))
    }
}
