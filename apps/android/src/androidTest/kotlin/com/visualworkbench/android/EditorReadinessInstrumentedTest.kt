package com.visualworkbench.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.visualworkbench.android.editor.EditorReadiness
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** No Activity, Compose host, frame clock, snapshot publication or native core.
 * The existing real camera ownership tests remain the integration regressions. */
@RunWith(AndroidJUnit4::class)
class EditorReadinessInstrumentedTest {
    @Test fun initializationReleaseWakesWithoutCompose() = runBlocking {
        val readiness = EditorReadiness()
        val waiter = async(start = CoroutineStart.UNDISPATCHED) { readiness.awaitReady() }
        assertFalse(waiter.isCompleted)
        readiness.busy(false)
        assertTrue(withTimeout(1_000) { waiter.await() })
    }

    @Test fun pendingEditMustSettleEvenAfterBusyClears() = runBlocking {
        val readiness = EditorReadiness()
        readiness.pending(2)
        val waiter = async(start = CoroutineStart.UNDISPATCHED) { readiness.awaitReady() }
        readiness.busy(false)
        yield(); assertFalse(waiter.isCompleted)
        readiness.pending(1)
        yield(); assertFalse(waiter.isCompleted)
        readiness.pending(0)
        assertTrue(withTimeout(1_000) { waiter.await() })
    }

    @Test fun publicationDoesNotReenterAnUnfinishedFinallyBlock() = runBlocking {
        val readiness = EditorReadiness()
        var publisherFinished = false
        val waiter = async(start = CoroutineStart.UNDISPATCHED) {
            readiness.awaitReady().also { assertTrue(publisherFinished) }
        }
        readiness.busy(false)
        publisherFinished = true
        assertTrue(withTimeout(1_000) { waiter.await() })
    }

    @Test fun interveningOperationRevokesAnUnconsumedIdleOffer() = runBlocking {
        val readiness = EditorReadiness()
        val waiter = async(start = CoroutineStart.UNDISPATCHED) { readiness.awaitReady() }
        readiness.busy(false)
        readiness.busy(true)
        yield(); assertFalse(waiter.isCompleted)
        readiness.busy(false)
        assertTrue(withTimeout(1_000) { waiter.await() })
    }

    @Test fun disposalRefusesAnIdleOfferAndAllFutureRequests() = runBlocking {
        val readiness = EditorReadiness()
        readiness.busy(false)
        val waiter = async(start = CoroutineStart.UNDISPATCHED) { readiness.awaitReady() }
        readiness.close()
        assertFalse(withTimeout(1_000) { waiter.await() })
        readiness.busy(false); readiness.pending(0)
        assertFalse(readiness.awaitReady())
    }

    @Test fun cancelledWaiterDoesNotConsumeTheNextIdleOffer() = runBlocking {
        val readiness = EditorReadiness()
        val cancelled = async(start = CoroutineStart.UNDISPATCHED) { readiness.awaitReady() }
        cancelled.cancelAndJoin()
        val next = async(start = CoroutineStart.UNDISPATCHED) { readiness.awaitReady() }
        readiness.busy(false)
        assertTrue(withTimeout(1_000) { next.await() })
    }
}
