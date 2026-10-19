package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class DiagnosticsPrivacyAuditTest {
    private val maximum: Long = 9_007_199_254_740_991L
    // Raw fixture fragments exist only in these fake adversarial tests. There
    // is no production free-text report constructor or redaction API.
    private fun fixture(platform: String = "\"android\"", startup: String = "null", heap: String = "null",
        samples: String = "null", decoded: String = "null", dropped: String = "null", echo: String = "null",
        rotated: String = "0", events: String = ""): String =
        "{\"schema_version\":1,\"platform\":$platform,\"metrics\":{\"startup_ms\":$startup,\"heap_used_bytes\":$heap,\"trace_samples\":$samples,\"frames_decoded\":$decoded,\"frames_dropped\":$dropped,\"echo_rtt_ms\":$echo},\"events_rotated\":$rotated,\"events\":[$events]}\n"
    private fun event(time: String = "0", code: String = "\"app_started\""): String =
        "{\"elapsed_ms\":$time,\"code\":$code}"
    private fun rejected(text: String, count: Int = 0, rotated: Long = 0L): Unit {
        assertFalse(diagnosticsPrivacyAudit(text, count, rotated))
    }

    @Test fun everyRealFactoryPlatformAndCodePassesWithNullOrMaximumMetrics() {
        for (platform in DiagnosticPlatform.entries) {
            for (code in DiagnosticEventCode.entries) {
                val log = PrivateDiagnosticLog(1)
                log.append(0L, code)
                log.append(maximum, code)
                for (metrics in listOf(DiagnosticMetrics(), DiagnosticMetrics(
                    startupMillis = maximum, heapUsedBytes = maximum,
                    traceSampleCount = Int.MAX_VALUE, framesDecoded = maximum,
                    framesDropped = maximum, echoRoundTripMillis = maximum))) {
                    val report = log.prepare(platform, metrics)
                    assertTrue(diagnosticsPrivacyAudit(report.jsonText, report.eventCount, report.rotatedEventCount))
                }
            }
        }
    }

    @Test fun duplicateAndUnknownKeysAreRejectedAtEveryObjectLevel() {
        val base = fixture(events = event())
        val malformed = listOf(
            base.replace("\"platform\":\"android\"", "\"platform\":\"android\",\"platform\":\"windows\""),
            base.replace("\"schema_version\":1", "\"schema_version\":1,\"schema_version\":1"),
            base.replace("\"metrics\":{", "\"metrics\":{},\"metrics\":{"),
            base.replace("\"startup_ms\":null", "\"startup_ms\":null,\"startup_ms\":0"),
            base.replace("\"heap_used_bytes\":null", "\"heap_used_bytes\":0,\"heap_used_bytes\":null"),
            base.replace("\"events_rotated\":0", "\"events_rotated\":0,\"events_rotated\":0"),
            base.replace("\"events\":[", "\"events\":[],\"events\":["),
            base.replace("\"elapsed_ms\":0", "\"elapsed_ms\":0,\"elapsed_ms\":0"),
            base.replace("\"code\":\"app_started\"", "\"code\":\"app_started\",\"code\":\"trace_saved\""),
            base.replace("\"metrics\":{", "\"account\":\"fake-account-for-tests\",\"metrics\":{"),
            base.replace("\"startup_ms\":null", "\"window_title\":\"fake-window-title\",\"startup_ms\":null"),
            base.replace("\"elapsed_ms\":0", "\"serial\":\"fake-device-for-tests\",\"elapsed_ms\":0"),
        )
        for (text in malformed) rejected(text, 1)
    }

    @Test fun escapedKeysAlternateOrderAndNoncanonicalRepresentationsCannotAliasTheSchema() {
        val base = fixture()
        val malformed = listOf(
            base.replace("schema_version", "schema_\\u0076ersion"),
            base.replace("startup_ms", "startup_\\u006ds"),
            base.replace("\"android\"", "\"\\u0061ndroid\""),
            base.replace("\"schema_version\":1", "\"schema_version\":1.0"),
            base.replace("\"schema_version\":1", "\"schema_version\":1e0"),
            base.replace("\"schema_version\":1", "\"schema_version\":01"),
            base.replace("\"schema_version\":1,\"platform\":\"android\"", "\"platform\":\"android\",\"schema_version\":1"),
            base.replace("\"metrics\":{", "\"metrics\": {"),
            " $base", base + " ", base + "\n", base.dropLast(1),
        )
        // The auditor is for the existing canonical writer, not a general JSON importer.
        for (text in malformed) rejected(text)
    }

    @Test fun syntheticIdentityCredentialAndPathStringsNeverFitKnownValuePositions() {
        val fakeValues = listOf("\"fake-token-for-tests-only\"", "\"unit-user@invalid.example\"",
            "\"C:\\\\FAKE\\\\private.txt\"", "\"00000000-0000-4000-8000-000000000000\"",
            "\"https://invalid.example/unit-test\"")
        for (value in fakeValues) {
            rejected(fixture(platform = value))
            rejected(fixture(startup = value))
            rejected(fixture(heap = value))
            rejected(fixture(samples = value))
            rejected(fixture(decoded = value))
            rejected(fixture(dropped = value))
            rejected(fixture(echo = value))
            rejected(fixture(rotated = value))
            rejected(fixture(events = event(time = value)), 1)
            rejected(fixture(events = event(code = value)), 1)
        }
        rejected(fixture().replace("\"metrics\":{", "\"credential\":\"fake-token-for-tests-only\",\"metrics\":{"))
    }

    @Test fun stringAndSchemaTypesAreClosedRatherThanCoerced() {
        for (wrong in listOf("true", "false", "null", "[]", "{}", "42", "\"ANDROID\"", "\"android/windows\""))
            rejected(fixture(platform = wrong))
        for (wrong in listOf("true", "false", "{}", "[]", "\"0\"", "\"null\"")) {
            rejected(fixture(startup = wrong))
            rejected(fixture(samples = wrong))
            rejected(fixture(rotated = wrong))
            rejected(fixture(events = event(time = wrong)), 1)
        }
        for (wrong in listOf("null", "true", "42", "[]", "{}", "\"trace_savEd\"", "\"app_started extra\""))
            rejected(fixture(events = event(code = wrong)), 1)
        for (wrong in listOf("0", "2", "-1", "null", "true", "\"1\""))
            rejected(fixture().replace("\"schema_version\":1", "\"schema_version\":$wrong"))
    }

    @Test fun negativeNonfiniteFractionalExponentAndOverflowCountersAreRejected() {
        val wrong = listOf("-1", "-0", "+0", "+1", "00", "01", "1.0", ".1", "1.", "1e0", "1E3", "1e309",
            "NaN", "Infinity", "-Infinity", "9007199254740992", "9223372036854775807", "18446744073709551616")
        for (value in wrong) {
            rejected(fixture(startup = value))
            rejected(fixture(heap = value))
            rejected(fixture(decoded = value))
            rejected(fixture(dropped = value))
            rejected(fixture(echo = value))
            rejected(fixture(rotated = value))
            rejected(fixture(events = event(time = value)), 1)
        }
        rejected(fixture(samples = "2147483648"))
        assertTrue(diagnosticsPrivacyAudit(fixture(startup = "0", heap = "$maximum", samples = "2147483647",
            decoded = "0", dropped = "0", echo = "0", rotated = "$maximum"), 0, maximum))
    }

    @Test fun eventCountAndRotationMetadataMustMatchTheExactBody() {
        val empty = fixture()
        rejected(empty, -1); rejected(empty, 1); rejected(empty, 257)
        rejected(empty, rotated = -1L); rejected(empty, rotated = maximum + 1L)
        rejected(empty, rotated = 1L)
        rejected(fixture(rotated = "1"))
        val one = fixture(events = event())
        rejected(one, 0); rejected(one, 2)
        assertTrue(diagnosticsPrivacyAudit(one, 1, 0L))
    }

    @Test fun eventsMustBeMonotonicButEqualTimesAreValid() {
        assertTrue(diagnosticsPrivacyAudit(fixture(events = event("7") + "," + event("7", "\"report_prepared\"")), 2, 0L))
        rejected(fixture(events = event("7") + "," + event("6", "\"report_prepared\"")), 2)
        rejected(fixture(events = event("7") + "," + event("0")), 2)
    }

    @Test fun maximumEventCountPassesAndAnExtraEventCannotHideBehindMetadata() {
        val full = List(256) { event(it.toString()) }.joinToString(",")
        assertTrue(diagnosticsPrivacyAudit(fixture(events = full), 256, 0L))
        rejected(fixture(events = full + "," + event("256")), 256)
        rejected(fixture(events = full), 255)
    }

    @Test fun rawControlsUnicodeBomAndEscapedAliasesAreRejected() {
        for (character in (0..31).map { Char(it) } + listOf(Char(127), Char(128), '\uD800', '\uFFFF')) {
            rejected(fixture(platform = "\"android$character\""))
            rejected(fixture(events = event(code = "\"app_started$character\"")), 1)
        }
        rejected("\uFEFF" + fixture())
        rejected(fixture(events = event(code = "\"app_\\u0073tarted\"")), 1)
        rejected(fixture(events = event(code = "\"app_started\\n\"")), 1)
    }

    @Test fun boundedMalformedDeepAndTruncatedInputsRefuseWithoutARecursiveParser() {
        rejected(" ".repeat(65_536)); rejected(" ".repeat(65_537))
        rejected(fixture(startup = "9".repeat(60_000)))
        rejected(fixture(startup = "[".repeat(5000) + "0" + "]".repeat(5000)))
        val full = fixture(events = event())
        for (length in full.indices) rejected(full.take(length), 1)
        assertTrue(diagnosticsPrivacyAudit(full, 1, 0L))
    }

    @Test fun trailingContentTruncatedStructuresAndArrayTypeChangesAreRejected() {
        val malformed = listOf(
            fixture() + fixture() to 0, fixture() + "\u0000" to 0,
            fixture().replace("\"events\":[]", "\"events\":{}") to 0,
            fixture(events = event() + ",") to 1, fixture(events = "[" + event() + "]") to 1,
            fixture(events = "null") to 1,
            fixture(events = event()).replace("\"code\":", "\"code\" ") to 1,
            fixture(events = event()).replace("0,\"code\"", "0 \"code\"") to 1,
        )
        for ((text, count) in malformed) rejected(text, count)
    }
}
