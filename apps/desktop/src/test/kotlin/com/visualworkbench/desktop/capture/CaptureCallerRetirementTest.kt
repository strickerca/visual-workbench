package com.visualworkbench.desktop.capture

import com.visualworkbench.desktop.DesktopDiagnosticsLifecycle
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class CaptureCallerRetirementTest {
    private fun assertReportedOriginal(expected: Throwable, actual: Throwable?) {
        val reported = checkNotNull(actual)
        assertEquals(expected.javaClass, reported.javaClass)
        // Recovery may copy the exception once at a coroutine boundary.
        assertSame(expected, if (reported === expected) reported else reported.cause)
    }

    @Test fun verificationServiceRefusalKeepsEditorAndNativeParentsAliveForRetry() = runBlocking {
        val calls = mutableListOf<String>(); var refuses = true
        val failure = IllegalStateException("synthetic service retirement refusal")
        suspend fun close() = withRetiredVerificationAccess({ calls += "requests" }, {
            calls += "service"; if (refuses) throw failure
        }) { calls += "editor"; calls += "native parents" }
        assertReportedOriginal(failure, runCatching { close() }.exceptionOrNull())
        assertEquals(listOf("requests", "service"), calls)
        refuses = false; close()
        assertEquals(listOf("requests", "service", "requests", "service", "editor", "native parents"), calls)
    }

    @Test fun failedRequestJoinDoesNotCloseServiceOrEditor() = runBlocking {
        val calls = mutableListOf<String>(); val failure = IllegalStateException("synthetic request join refusal")
        val result = runCatching { withRetiredVerificationAccess({ throw failure },
            { calls += "service" }) { calls += "editor" } }
        assertReportedOriginal(failure, result.exceptionOrNull()); assertTrue(calls.isEmpty())
    }

    @Test fun cancelledShutdownJoinsVerificationServiceBeforeEditorAndNativeParents() = runBlocking {
        val entered = CompletableDeferred<Unit>(); val settle = CompletableDeferred<Unit>()
        val calls = mutableListOf<String>()
        val task = launch(start = CoroutineStart.UNDISPATCHED) {
            withRetiredVerificationAccess({ calls += "requests" }, {
                entered.complete(Unit); settle.await(); calls += "service"
            }) { calls += "editor"; calls += "native parents" }
        }
        try {
            entered.await(); task.cancel(); assertFalse(task.isCompleted)
            assertEquals(listOf("requests"), calls)
            settle.complete(Unit); withTimeout(2000) { task.join() }
            assertEquals(listOf("requests", "service", "editor", "native parents"), calls)
        } finally { settle.complete(Unit); withContext(NonCancellable) { task.cancelAndJoin() } }
    }
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
