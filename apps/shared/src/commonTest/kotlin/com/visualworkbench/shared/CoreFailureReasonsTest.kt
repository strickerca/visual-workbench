package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class CoreFailureReasonsTest {
    private fun failure(sequence: Long, name: String) =
        CoreEvent(sequence, sequence, CoreEventKind.Failure, "store", name, 0)

    @Test fun aNewReasonSelectsTheSpecificMessageAndIsUsedOnlyOnce() {
        val events = mutableListOf(failure(1, "low disk space"))
        val reasons = CoreFailureReasons { after -> CoreEventLog(events.filter { it.sequence > after }, 0) }
        val general = ErrorCatalog.core(CoreFailureKind.Storage)
        val specific = reasons.advice(CoreFailureKind.Storage)
        assertEquals("This device is out of storage space.", specific.whatHappened)
        // The same old reason must not explain the next, unrelated failure.
        assertEquals(ErrorCatalog.core(CoreFailureKind.Invalid), reasons.advice(CoreFailureKind.Invalid))
        events += failure(2, "store busy")
        assertEquals("The project is open somewhere else.", reasons.advice(CoreFailureKind.Storage).whatHappened)
        assertEquals(general, reasons.advice(CoreFailureKind.Storage))
    }

    @Test fun withoutReadableEventsTheGeneralMessageIsUsedAndTheReasonIsNeverShown() {
        val unreadable = CoreFailureReasons { null }
        val text = unreadable.text(CoreFailure(CoreFailureKind.Worker))
        val general = ErrorCatalog.core(CoreFailureKind.Worker)
        assertEquals("${general.whatHappened} ${general.whatToDo}", text)
        val unknown = CoreFailureReasons { CoreEventLog(listOf(failure(5, "some internal reason")), 0) }
        assertTrue("some internal reason" !in unknown.text(CoreFailure(CoreFailureKind.Worker)))
    }
}
