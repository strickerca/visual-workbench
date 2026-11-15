package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNotEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

class CoreEventsTest {
    private fun event(sequence: Long, kind: CoreEventKind, name: String, value: Long = 0L) =
        CoreEvent(sequence, sequence * 10L, kind, "store", name, value)

    @Test fun eventsRefuseAnythingThatCouldBeRunTimeText() {
        for (text in listOf("", "C:\\Users\\name", "/home/name/file.png", "Mixed Case", "a".repeat(65), "tab\there", "naïve")) {
            assertFailsWith<IllegalArgumentException> { CoreEvent(1, 0, CoreEventKind.Failure, "store", text, 0) }
            assertFailsWith<IllegalArgumentException> { CoreEvent(1, 0, CoreEventKind.Failure, text, "cause", 0) }
        }
        assertFailsWith<IllegalArgumentException> { CoreEvent(0, 0, CoreEventKind.Failure, "store", "cause", 0) }
        assertFailsWith<IllegalArgumentException> { CoreEvent(1, -1, CoreEventKind.Failure, "store", "cause", 0) }
    }

    @Test fun logOrdersEventsAndReportsTheNewestFailure() {
        val log = CoreEventLog(
            listOf(
                event(1, CoreEventKind.Failure, "file not found"),
                event(2, CoreEventKind.DurationMicros, "image", 12_345),
                event(3, CoreEventKind.Failure, "low disk space"),
                event(4, CoreEventKind.Count, "image bytes", 2048),
            ),
            dropped = 5,
        )
        assertEquals("low disk space", log.latestFailure()?.name)
        assertEquals(
            listOf(
                "0.010 s  store: file not found",
                "0.020 s  store: image took 12.345 ms",
                "0.030 s  store: low disk space",
                "0.040 s  store: image bytes = 2048",
            ),
            log.reportLines(),
        )
        assertNull(CoreEventLog(emptyList(), 0).latestFailure())
        assertFailsWith<IllegalArgumentException> {
            CoreEventLog(listOf(event(2, CoreEventKind.Count, "a"), event(2, CoreEventKind.Count, "b")), 0)
        }
    }

    @Test fun catalogGivesEveryFailureAPlainMessageAndAnAction() {
        for (kind in CoreFailureKind.entries) {
            val advice = ErrorCatalog.core(kind)
            assertTrue(advice.whatHappened.isNotBlank() && advice.whatToDo.isNotBlank(), kind.name)
        }
        for (kind in SessionFailureKind.entries) {
            val advice = ErrorCatalog.session(kind)
            assertTrue(advice.whatHappened.isNotBlank() && advice.whatToDo.isNotBlank(), kind.name)
        }
        val internalWords = listOf("admission", "retire", "invalid", "backpressure", "worker", "raster", "lease")
        val all = CoreFailureKind.entries.map { ErrorCatalog.core(it) } + SessionFailureKind.entries.map { ErrorCatalog.session(it) }
        for (advice in all) {
            val text = (advice.whatHappened + " " + advice.whatToDo).lowercase()
            assertTrue(internalWords.none { it in text }, text)
        }
    }

    @Test fun aKnownCauseSelectsAMoreSpecificMessage() {
        val generic = ErrorCatalog.core(CoreFailureKind.Storage)
        val full = ErrorCatalog.core(CoreFailureKind.Storage, "low disk space")
        assertNotEquals(generic, full)
        assertTrue("space" in full.whatHappened)
        assertEquals(generic, ErrorCatalog.core(CoreFailureKind.Storage, "some unknown cause"))
        assertTrue("layer" in ErrorCatalog.core(CoreFailureKind.Invalid, "exhausted order interval").whatHappened)
    }

    /** The causes the core records for connection attempts (areas `session`
     * and `discovery`). Each must be valid event text and lead to plain advice. */
    private val connectionCauses = listOf(
        "no known route", "route did not answer in time", "route did not prove the paired device", "route unreachable",
        "route busy", "route handshake rejected", "paired device revoked or unknown", "listener bind failed",
        "incoming device not verified", "incoming handshake timed out", "incoming connection dropped",
        "trust store unavailable", "incoming handshake rejected", "incoming hello timed out", "upgrade required",
        "paired device revoked or not verified", "project storage failed during sync", "link capacity exceeded",
        "paired device silent too long", "paired device sent an invalid message", "link closed", "link lost",
        "discovery network unavailable", "advertising could not start", "browsing could not start", "discovery failed",
        "discovery timed out", "too many discoveries open",
    )

    @Test fun everyConnectionCauseHasItsOwnPlainAdvice() {
        val generic = ErrorCatalog.session(SessionFailureKind.Transport)
        val internalWords = listOf("admission", "retire", "invalid", "backpressure", "worker", "raster", "lease", "route", "handshake", "listener")
        for (cause in connectionCauses) {
            assertTrue(isCoreEventText(cause), cause)
            val advice = ErrorCatalog.session(SessionFailureKind.Transport, cause)
            assertNotEquals(generic, advice, cause)
            val text = (advice.whatHappened + " " + advice.whatToDo).lowercase()
            assertTrue(advice.whatHappened.isNotBlank() && advice.whatToDo.isNotBlank() && internalWords.none { it in text }, text)
        }
        // A cause the catalog does not know, or none, keeps the general message.
        assertEquals(generic, ErrorCatalog.session(SessionFailureKind.Transport, "discovery record rejected"))
        assertEquals(generic, ErrorCatalog.session(SessionFailureKind.Transport, null))
        assertEquals(ErrorCatalog.session(SessionFailureKind.Expired), ErrorCatalog.session(SessionFailureKind.Expired, "unknown"))
        assertTrue("not been found" in ErrorCatalog.session(SessionFailureKind.Transport, "no known route").whatHappened)
        assertTrue("not the one you paired" in ErrorCatalog.session(SessionFailureKind.Transport, "route did not prove the paired device").whatHappened)
    }

    @Test fun aLiveConnectionsFailureWordLeadsToTheSameAdvice() {
        assertEquals(ErrorCatalog.session(SessionFailureKind.Transport, "no known route"), ErrorCatalog.link("no_known_route"))
        assertEquals(ErrorCatalog.session(SessionFailureKind.Transport, "upgrade required"), ErrorCatalog.link("upgrade_required"))
        assertEquals(ErrorCatalog.session(SessionFailureKind.Transport, "paired device revoked or unknown"), ErrorCatalog.link("untrusted_or_revoked"))
        for (word in listOf("timeout", "connection", "storage", "capacity")) assertTrue(ErrorCatalog.link(word) != null, word)
        for (word in listOf(null, "", "closed", "invalid_peer_message", "something new")) assertNull(ErrorCatalog.link(word))
    }
}
