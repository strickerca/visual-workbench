package com.visualworkbench.desktop.capture

import com.visualworkbench.desktop.DesktopDiagnosticsLifecycle
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class CaptureCallerRetirementTest {
    @Test fun abortedDepartureDoesNotPermanentlyRetireCapture() = runBlocking {
        for (diagnosticsReady in listOf(false, true)) {
            val calls = mutableListOf<String>()
            val diagnostics = DesktopDiagnosticsLifecycle({ false }, { true }, { diagnosticsReady })
            val closed = diagnostics.closeParentsAfterRetirement(admit = { false }, closeParents = {
                withRetiredCaptureCallers({ calls += "picker" }, { calls += "foreground" }) { calls += "parents" }
            })
            assertFalse(closed)
            assertTrue(calls.isEmpty())
        }
    }

    @Test fun eitherCaptureRefusalKeepsParentsAliveAndRetainsTheOriginalFailure() = runBlocking {
        for (failed in listOf("picker", "foreground")) {
            val calls = mutableListOf<String>()
            val failure = IllegalStateException("unknown source retirement")
            val retained = PickerReleaseAction { calls += failed; throw failure }
            repeat(2) {
                val result = runCatching {
                    withRetiredCaptureCallers(
                        { if (failed == "picker") retained.close() else calls += "picker" },
                        { if (failed == "foreground") retained.close() else calls += "foreground" },
                    ) { calls += "parents" }
                }
                val reported = checkNotNull(result.exceptionOrNull())
                assertEquals(failure.javaClass, reported.javaClass)
                assertEquals(failure.message, reported.message)
                // Coroutine recovery may copy the reported Throwable. Its
                // cause must still identify the exact retained original.
                assertSame(failure, if (reported === failure) reported else reported.cause)
                // No suspension boundary here: the owner must retain the
                // original failure rather than credit another destruction.
                assertSame(failure, runCatching { retained.close() }.exceptionOrNull())
            }
            assertEquals(1, calls.count { it == failed })
            assertFalse("parents" in calls)
            if (failed == "picker") assertFalse("foreground" in calls)
        }
    }

    @Test fun cancelledQuitStillJoinsBothSourcesBeforeClosingAnyParent() = runBlocking {
        val calls = mutableListOf<String>()
        val entered = CompletableDeferred<Unit>()
        val pickerGate = CompletableDeferred<Unit>()
        val foregroundEntered = CompletableDeferred<Unit>()
        val foregroundGate = CompletableDeferred<Unit>()
        val close = launch(start = CoroutineStart.UNDISPATCHED) {
            withRetiredCaptureCallers(
                { entered.complete(Unit); pickerGate.await(); calls += "picker" },
                { foregroundEntered.complete(Unit); foregroundGate.await(); calls += "foreground" },
            ) { calls += "parents" }
        }
        try {
            entered.await(); close.cancel()
            assertFalse(close.isCompleted); assertTrue(calls.isEmpty())
            pickerGate.complete(Unit)
            withTimeout(2000) { foregroundEntered.await() }
            assertEquals(listOf("picker"), calls); assertFalse(close.isCompleted)
            foregroundGate.complete(Unit)
            withTimeout(2000) { close.join() }
            assertEquals(listOf("picker", "foreground", "parents"), calls)
        } finally {
            pickerGate.complete(Unit); foregroundGate.complete(Unit)
            withContext(NonCancellable) { close.cancelAndJoin() }
        }
    }
}
