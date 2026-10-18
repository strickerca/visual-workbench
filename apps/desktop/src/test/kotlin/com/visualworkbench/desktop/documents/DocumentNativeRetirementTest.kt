package com.visualworkbench.desktop.documents

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class DocumentNativeRetirementTest {
    private suspend fun isolation(block: suspend () -> Unit) {
        try { block(); fail("expected fixed failure") }
        catch (error: DocumentFailure) { assertEquals(DocumentFailureKind.Isolation, error.kind) }
    }

    @Test fun everyAttemptedRunNeedsProofBeforeTokenRelease() = runBlocking {
        for (outcome in listOf("success", "worker", "cancelled", "isolation")) {
            val events = mutableListOf<String>()
            val owner = DocumentNativeRetirement({ events += "proof" }, { events += "destroy" })
            owner.beforeRun()
            events += outcome
            owner.retire(); owner.retire()
            assertEquals(listOf(outcome, "proof", "destroy"), events)
        }
    }

    @Test fun failedProofRetainsSameTokenForRetry() = runBlocking {
        var proofs = 0; var destroyed = 0
        val owner = DocumentNativeRetirement({ if (++proofs == 1) error("private path") }, { destroyed++ })
        owner.beforeRun()
        isolation { owner.retire() }; assertEquals(0, destroyed)
        owner.retire(); owner.retire()
        assertEquals(2, proofs); assertEquals(1, destroyed)
    }

    @Test fun failedTokenReleaseRequiresNewProofAndSameOwner() = runBlocking {
        var proofs = 0; var destroys = 0
        val owner = DocumentNativeRetirement({ proofs++ }, { if (++destroys == 1) error("synthetic") })
        owner.beforeRun()
        isolation { owner.retire() }
        owner.retire(); assertEquals(2, proofs); assertEquals(2, destroys)
    }

    @Test fun cancellationDuringProofDoesNotBlockAndFailedSignalReproves() = runBlocking {
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        var proofs = 0; var destroyed = 0; var signals = 0
        val owner = DocumentNativeRetirement({
            proofs++; if (proofs == 1) { entered.complete(Unit); release.await() }
        }, { destroyed++ })
        owner.beforeRun()
        val retirement = launch { owner.retire() }
        withTimeout(1000) { entered.await() }
        owner.cancel { signals++; error("synthetic") }
        owner.cancel { signals++ }
        assertEquals(0, destroyed)
        release.complete(Unit); retirement.join()
        assertEquals(1, signals); assertEquals(2, proofs); assertEquals(1, destroyed)
    }

    @Test fun cancelledUndispatchedCallCannotStartAndRetiresWithoutWorkerProof() = runBlocking {
        var proofs = 0; var destroyed = 0
        val owner = DocumentNativeRetirement({ proofs++ }, { destroyed++ })
        owner.cancel {}
        try { owner.beforeRun(); fail("cancelled owner dispatched") }
        catch (error: DocumentFailure) { assertEquals(DocumentFailureKind.Cancelled, error.kind) }
        owner.retire(); owner.cancel { fail("retired token signalled") }
        assertEquals(0, proofs); assertEquals(1, destroyed)
    }

    @Test fun runtimeCloseProofRunsOnceAndPinsCanRetryAfterRuntimeRelease() = runBlocking {
        val events = mutableListOf<String>()
        val runtime = DocumentNativeRetirement({ events += "proof" }, { events += "runtime" }, initiallyRequired = true)
        runtime.retire(); events += "failed pins"
        runtime.retire(); events += "pins"
        assertEquals(listOf("proof", "runtime", "failed pins", "pins"), events)
    }

    @Test fun failedCancellationSignalNeedsProofEvenWithoutDispatch() = runBlocking {
        val events = mutableListOf<String>()
        val owner = DocumentNativeRetirement({ events += "proof" }, { events += "destroy" })
        owner.cancel { error("synthetic") }
        owner.retire()
        assertEquals(listOf("proof", "destroy"), events)
    }
}
