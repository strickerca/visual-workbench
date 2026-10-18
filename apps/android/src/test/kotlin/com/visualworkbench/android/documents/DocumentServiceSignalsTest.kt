package com.visualworkbench.android.documents

import com.visualworkbench.shared.*
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class DocumentServiceSignalsTest {
    @Test fun resultReadyIsNotProcessRetirement() {
        val signals = DocumentServiceSignals(); signals.dispatching(); signals.resultReady()
        assertTrue(signals.finished.isCompleted); assertFalse(signals.retired.isCompleted)
    }
    @Test fun normalDisconnectBetweenResultAndDeathDoesNotPoisonSuccess() {
        val signals = DocumentServiceSignals(); signals.dispatching(); signals.resultReady(); signals.disconnected()
        assertFalse(signals.failed.isCompleted); assertFalse(signals.retired.isCompleted)
        signals.processDied(); assertTrue(signals.retired.isCompleted)
    }
    @Test fun disconnectWithoutResultIsStillNotDeath() {
        val signals = DocumentServiceSignals(); signals.dispatching(); signals.disconnected()
        assertFalse(signals.retired.isCompleted); assertFalse(signals.finished.isCompleted)
    }
    @Test fun connectionFailureBeforeDispatchCannotClaimACompletedWorker() = runBlocking {
        val signals = DocumentServiceSignals(); signals.disconnected()
        assertFalse(signals.dispatched); assertFalse(signals.retired.isCompleted)
        assertEquals(DocumentFailureKind.Isolation, signals.failed.await())
    }
    @Test fun realWriterFailuresAreNotErasedByLaterDeath() = runBlocking {
        val signals = DocumentServiceSignals(); signals.dispatching(); signals.failure(DocumentFailureKind.Worker); signals.processDied()
        assertTrue(signals.retired.isCompleted); assertEquals(DocumentFailureKind.Worker, signals.failed.await())
    }
}
