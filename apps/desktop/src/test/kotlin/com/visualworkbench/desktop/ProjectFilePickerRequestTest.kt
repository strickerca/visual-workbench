package com.visualworkbench.desktop

import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Path

class ProjectFilePickerRequestTest {
    private val selected = Path.of("synthetic-selection.png")
    private class Dialog

    @Test fun staleQueuedRequestNeverConstructsOrPublishesAndStillReleasesItsLease() {
        var created = 0
        var accepted = 0
        var released = 0
        val request = request { released++ }
        request.choose({ false }, { created++; Dialog() }, { selected }, {}, { accepted++ })
        assertEquals(0, created)
        assertEquals(0, accepted)
        assertFalse(request.returnEligible)
        assertEquals(0, released)
        request.close()
        assertTrue(request.retired)
        assertEquals(1, released)
    }

    @Test fun changedAdmissionDuringNativeLoopRefusesTheResultAndConnectionReturn() {
        var current = true
        var accepted = 0
        var disposed = 0
        val request = request {}
        request.choose({ current }, { Dialog() }, { current = false; selected }, { disposed++ }, { accepted++ })
        assertEquals(0, accepted)
        assertFalse(request.returnEligible)
        request.close()
        assertEquals(1, disposed)
    }

    @Test fun changedAdmissionDuringConstructionDisposesWithoutShowing() {
        var current = true
        var shown = 0
        var disposed = 0
        val request = request {}
        request.choose({ current }, { current = false; Dialog() }, { shown++; selected }, { disposed++ }, { fail("stale result") })
        request.close()
        assertEquals(0, shown)
        assertEquals(1, disposed)
        assertFalse(request.returnEligible)
    }

    @Test fun retirementDuringConstructionHoldsFocusUntilTheReturnedDialogIsDisposed() {
        var released = 0
        var disposed = 0
        val request = request { released++ }
        request.choose({ true }, {
            request.close()
            assertFalse(request.retired)
            assertEquals(0, released)
            Dialog()
        }, { fail("retired dialog must not show"); null }, { disposed++ }, { fail("late result") })
        assertTrue(request.retired)
        assertEquals(1, disposed)
        assertEquals(1, released)
    }

    @Test fun cancelRetiresInOrderWithoutPublishingAndKeepsAnEligibleReturnRoute() {
        val events = mutableListOf<String>()
        val request = request { events += "focus released" }
        request.choose({ true }, { Dialog() }, { null }, { events += "native disposed" }, { events += "published" })
        assertTrue(request.returnEligible)
        assertTrue(events.isEmpty())
        request.close()
        request.close()
        assertEquals(listOf("native disposed", "focus released"), events)
        assertTrue(request.retired)
    }

    @Test fun closeInsideTheNativeLoopPreventsALateResultAndIsIdempotent() {
        var disposed = 0
        var released = 0
        var accepted = 0
        val request = request { released++ }
        request.choose({ true }, { Dialog() }, { request.close(); selected }, { disposed++ }, { accepted++ })
        request.close()
        assertEquals(0, accepted)
        assertFalse(request.returnEligible)
        assertEquals(1, disposed)
        assertEquals(1, released)
    }

    @Test fun failedNativeDisposalHoldsFocusAndRetriesTheSameOwner() {
        var disposals = 0
        var released = 0
        val request = request { released++ }
        request.choose({ true }, { Dialog() }, { null }, {
            disposals++
            if (disposals == 1) error("synthetic disposal failure")
        }, {})
        assertTrue(runCatching { request.close() }.isFailure)
        assertFalse(request.retired)
        assertEquals(0, released)
        request.close()
        assertTrue(request.retired)
        assertEquals(2, disposals)
        assertEquals(1, released)
    }

    @Test fun failedCloseInsideTheModalLoopStillFencesLateSelectionAndReturn() {
        for (failDisposal in listOf(false, true)) {
            var disposals = 0
            var releases = 0
            var accepted = 0
            val request = request {
                releases++
                if (!failDisposal && releases == 1) error("synthetic lease failure")
            }
            request.choose({ true }, { Dialog() }, {
                assertTrue(runCatching { request.close() }.isFailure)
                assertFalse(request.retired)
                selected
            }, {
                disposals++
                if (failDisposal && disposals == 1) error("synthetic disposal failure")
            }, { accepted++ })
            assertEquals(0, accepted)
            assertFalse(request.returnEligible)
            request.close()
            assertTrue(request.retired)
            assertEquals(if (failDisposal) 2 else 1, disposals)
            assertEquals(if (failDisposal) 1 else 2, releases)
        }
    }

    @Test fun failedCloseBeforeTheQueuedTickCannotAdmitConstructionLater() {
        var releases = 0
        var created = 0
        val request = request { if (++releases == 1) error("synthetic lease failure") }
        assertTrue(runCatching { request.close() }.isFailure)
        request.choose({ true }, { created++; Dialog() }, { selected }, {}, { fail("retired request") })
        assertEquals(0, created)
        assertFalse(request.returnEligible)
        request.close()
        assertTrue(request.retired)
    }

    @Test fun failedFocusReleaseDoesNotRedisposeTheNativeOwnerOnRetry() {
        var releases = 0
        var disposed = 0
        val request = request { if (++releases == 1) error("synthetic lease failure") }
        request.choose({ true }, { Dialog() }, { null }, { disposed++ }, {})
        assertTrue(runCatching { request.close() }.isFailure)
        assertFalse(request.retired)
        request.close()
        assertTrue(request.retired)
        assertEquals(1, disposed)
        assertEquals(2, releases)
    }

    @Test fun reentrantNativeDisposalCannotDoubleReleaseFocus() {
        var disposed = 0
        var released = 0
        val request = request { released++ }
        request.choose({ true }, { Dialog() }, { null }, { disposed++; request.close() }, {})
        request.close()
        assertTrue(request.retired)
        assertEquals(1, disposed)
        assertEquals(1, released)
    }

    @Test fun constructionAndShowFailuresKeepCleanupOwnedAndMayReturnOnlyWhileCurrent() {
        for (duringShow in listOf(false, true)) {
            var released = 0
            var disposed = 0
            val request = request { released++ }
            assertTrue(runCatching {
                request.choose<Dialog>({ true }, {
                    if (!duringShow) error("synthetic construction failure")
                    Dialog()
                }, { error("synthetic show failure") }, { disposed++ }, { fail("must not publish") })
            }.isFailure)
            assertTrue(request.returnEligible)
            request.close()
            assertTrue(request.retired)
            assertEquals(if (duringShow) 1 else 0, disposed)
            assertEquals(1, released)
        }
    }

    @Test fun retiredQueuedRequestAndRepeatedDispatchNeverPublishAgain() {
        var created = 0
        var accepted = 0
        val request = request {}
        repeat(2) {
            request.choose({ true }, { created++; Dialog() }, { selected }, {}, { accepted++ })
        }
        request.close()
        request.choose({ true }, { created++; Dialog() }, { selected }, {}, { accepted++ })
        assertEquals(1, created)
        assertEquals(1, accepted)
    }

    @Test fun acceptedDocumentConfirmationAcquiresItsLeaseBeforeThePickerReleasesFocus() {
        var held = 1
        val events = mutableListOf<Int>()
        val request = request { held--; events += held }
        request.choose({ true }, { Dialog() }, { selected }, {}, {
            held++ // The PDF/SVG confirmation becomes the next focus owner.
            events += held
        })
        request.close()
        assertEquals(listOf(2, 1), events)
        assertTrue(request.returnEligible)
        assertTrue(request.retired)
    }

    @Test fun acceptedOpenMayChangeTheProjectWithoutInvalidatingTheAlreadyCheckedReturn() {
        var current = true
        var accepted = 0
        val request = request {}
        request.choose({ current }, { Dialog() }, { selected }, {}, { current = false; accepted++ })
        assertEquals(1, accepted)
        assertTrue(request.returnEligible)
        request.close()
    }

    private fun request(release: () -> Unit): ProjectFilePickerRequest =
        ProjectFilePickerRequest(Any(), Any(), 7L, true, AutoCloseable(release))
}
