package com.visualworkbench.android

import com.visualworkbench.android.editor.*

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class AiServiceControllerInstrumentedTest {
    @Test fun cancelledLateAcquisitionRefusesReopenUntilItsProducerSettles() = runBlocking {
        val service = Service(); val gate = CompletableDeferred<Unit>(); val entered = CompletableDeferred<Unit>()
        val flags = mutableListOf<Boolean>()
        val owner = AiServiceController(this, "fixture-private") { _, initialize ->
            flags += initialize; entered.complete(Unit)
            withContext(NonCancellable) { gate.await() }; service
        }
        try {
            owner.activate(); withTimeout(2_000) { entered.await() }; owner.cancelSettings()
            owner.activate(); owner.initialize(); yield()
            assertEquals(listOf(false), flags); assertTrue(owner.state.value.busy)
            try { owner.requireService(); fail("Expected unsettled producer to remain Busy") }
            catch (failure: AiFailure) { assertEquals(AiFailureKind.Busy, failure.kind) }
            val closing = launch { owner.close() }; yield()
            assertFalse(closing.isCompleted); assertEquals(0, service.closes)
            gate.complete(Unit); closing.join()
            assertEquals(1, service.closes); assertTrue(owner.state.value.closed)
        } finally { gate.complete(Unit); withContext(NonCancellable) { owner.close() } }
    }
    @Test fun cancelledConfigurationRefusesNewActionsAndWipesRefusedKeyUntilJoined() = runBlocking {
        val service = Service(); val owner = AiServiceController(this, "fixture-private") { _, _ -> service }
        val gate = CompletableDeferred<Unit>()
        try {
            owner.activate(); idle(owner); service.configurationGate = gate
            owner.configure("{}", "policy-one")
            withTimeout(2_000) { while (service.configurations != 2) yield() }
            owner.cancelSettings(); owner.activate(); owner.configureDailyBudget("policy-one", 10u)
            val bytes = byteArrayOf(65, 66); owner.saveWindowsKey(bytes)
            assertTrue(bytes.all { it == 0.toByte() }); assertEquals(0, service.writes)
            assertNull(service.budgetWith); assertEquals(2, service.configurations); assertTrue(owner.state.value.busy)
            val closing = launch { owner.close() }; yield(); assertFalse(closing.isCompleted)
            gate.complete(Unit); closing.join(); assertEquals(1, service.closes)
        } finally { gate.complete(Unit); withContext(NonCancellable) { owner.close() } }
    }
    private class Service : WorkbenchAiService {
        var closes = 0
        var configurations = 0
        var writes = 0
        var configuredWith: Pair<String, String>? = null
        var budgetWith: Pair<String, ULong>? = null
        var configurationGate: CompletableDeferred<Unit>? = null
        var writeGate: CompletableDeferred<Unit>? = null
        var closeFailure: AiFailure? = null
        override suspend fun configuration(): AiConfiguration {
            configurations++
            configurationGate?.let { withContext(NonCancellable) { it.await() } }
            return AiConfiguration("{}", "policy-one", "2026-10-01", "2026-10-31", "fixture-model", "fixture-quality", 5_000_000u)
        }
        override suspend fun configure(json: String, expectedFingerprint: String): AiConfiguration {
            configuredWith = json to expectedFingerprint
            return configuration()
        }
        override suspend fun configureDailyBudget(expectedFingerprint: String, dailySoftBudgetMicrousd: ULong): AiConfiguration {
            budgetWith = expectedFingerprint to dailySoftBudgetMicrousd
            return configuration().copy(dailySoftBudgetMicrousd = dailySoftBudgetMicrousd)
        }
        override suspend fun readiness() = AiReadiness(true, true, true, 0u, 5_000_000u)
        override suspend fun saveWindowsKey(bytes: ByteArray) {
            writes++
            writeGate?.let { withContext(NonCancellable) { it.await() } }
        }
        override suspend fun removeWindowsKey() = Unit
        override suspend fun close() { closes++; closeFailure?.let { throw it } }
    }
    private suspend fun idle(owner: AiServiceController) {
        withTimeout(2_000) { while (owner.state.value.busy) yield() }
    }

    @Test fun constructionIsLazyAndOrdinaryActivationNeverProvisions() = runBlocking {
        val flags = mutableListOf<Boolean>()
        val service = Service()
        val owner = AiServiceController(this, "fixture-private") { _, initialize -> flags += initialize; service }
        assertTrue(flags.isEmpty())
        owner.activate(); idle(owner)
        assertEquals(listOf(false), flags)
        owner.close()
        assertEquals(1, service.closes)
    }
    @Test fun storageRefusalNeedsDistinctExplicitFirstUseAction() = runBlocking {
        val flags = mutableListOf<Boolean>()
        val service = Service()
        val owner = AiServiceController(this, "fixture-private") { _, initialize ->
            flags += initialize
            if (!initialize) throw AiFailure(AiFailureKind.Storage)
            service
        }
        owner.activate(); idle(owner)
        assertEquals(AiFailureKind.Storage, owner.state.value.failure)
        assertEquals(listOf(false), flags)
        owner.initialize(); idle(owner)
        assertEquals(listOf(false, true), flags)
        assertNotNull(owner.state.value.readiness)
        owner.close()
    }
    @Test fun cancelledLateAcquisitionIsClosedBeforeOwnerClosureCompletes() = runBlocking {
        val service = Service()
        val gate = CompletableDeferred<Unit>()
        val entered = CompletableDeferred<Unit>()
        val owner = AiServiceController(this, "fixture-private") { _, _ ->
            entered.complete(Unit)
            withContext(NonCancellable) { gate.await() }
            service
        }
        try {
        owner.activate(); withTimeout(2_000) { entered.await() }
        val closing = launch { owner.close() }
        yield()
        assertFalse(closing.isCompleted)
        gate.complete(Unit); closing.join()
        assertEquals(1, service.closes)
        assertTrue(owner.state.value.closed)
        assertNull(owner.state.value.configuration)
        assertNull(owner.state.value.readiness)
        } finally { gate.complete(Unit); withContext(NonCancellable) { owner.close() } }
    }
    @Test fun settingsMutationHasSingleAdmissionAndImmutableFingerprint() = runBlocking {
        val service = Service()
        val owner = AiServiceController(this, "fixture-private") { _, _ -> service }
        owner.activate(); idle(owner)
        val gate = CompletableDeferred<Unit>(); service.configurationGate = gate
        try {
        owner.configure("{\"first\":true}", "policy-one")
        yield()
        owner.configure("{\"second\":true}", "policy-two")
        assertEquals(AiFailureKind.Busy, owner.state.value.failure)
        gate.complete(Unit); idle(owner)
        assertEquals("{\"first\":true}" to "policy-one", service.configuredWith)
        } finally { gate.complete(Unit); withContext(NonCancellable) { owner.close() } }
    }
    @Test fun closingWaitsForASettingsProducerAndCannotRepopulateClearedState() = runBlocking {
        val service = Service()
        val owner = AiServiceController(this, "fixture-private") { _, _ -> service }
        owner.activate(); idle(owner)
        val gate = CompletableDeferred<Unit>(); service.configurationGate = gate
        try {
        owner.configure("{}", "policy-one")
        yield()
        val closing = launch { owner.close() }
        yield()
        assertFalse(closing.isCompleted)
        assertEquals(0, service.closes)
        gate.complete(Unit); closing.join()
        assertEquals(1, service.closes)
        assertNull(owner.state.value.configuration)
        assertTrue(owner.state.value.closed)
        } finally { gate.complete(Unit); withContext(NonCancellable) { owner.close() } }
    }
    @Test fun explicitKeyInputIsWipedOnInvalidBusyAndCancelledBranches() = runBlocking {
        val service = Service()
        val owner = AiServiceController(this, "fixture-private") { _, _ -> service }
        val invalid = byteArrayOf(10, 13)
        owner.saveWindowsKey(invalid); assertTrue(invalid.all { it == 0.toByte() })
        owner.activate(); idle(owner)
        val gate = CompletableDeferred<Unit>(); service.writeGate = gate
        try {
        val first = byteArrayOf(65, 66, 67)
        owner.saveWindowsKey(first); yield()
        val refused = byteArrayOf(68, 69)
        owner.saveWindowsKey(refused)
        assertTrue(refused.all { it == 0.toByte() })
        assertEquals(1, service.writes)
        owner.cancelSettings()
        gate.complete(Unit); idle(owner)
        assertTrue(first.all { it == 0.toByte() })
        } finally { gate.complete(Unit); withContext(NonCancellable) { owner.close() } }
    }
    @Test fun neverStartedKeyWorkStillWipesItsInput() = runBlocking {
        val job = Job(); job.cancel()
        val cancelledScope = CoroutineScope(coroutineContext + job)
        val owner = AiServiceController(cancelledScope, "fixture-private") { _, _ -> Service() }
        val bytes = byteArrayOf(65, 66)
        owner.saveWindowsKey(bytes)
        assertTrue(bytes.all { it == 0.toByte() })
        assertFalse(owner.state.value.busy)
        owner.close()
    }
    @Test fun closeIsIdempotentAndCannotStartNewWork() = runBlocking {
        val service = Service()
        val owner = AiServiceController(this, "fixture-private") { _, _ -> service }
        owner.activate(); idle(owner)
        owner.close(); owner.close(); owner.activate()
        assertEquals(1, service.closes)
        assertEquals(AiFailureKind.Closed, owner.state.value.failure)
    }
    @Test fun laterCloseCallersObserveTheSameSettledFailure() = runBlocking {
        val service=Service();val owner=AiServiceController(this,"fixture-private"){_,_->service}
        owner.activate();idle(owner);service.closeFailure=AiFailure(AiFailureKind.Storage)
        repeat(2){try{owner.close();fail("Expected retained closure failure")}catch(failure:AiFailure){assertSame(service.closeFailure,failure)}}
        assertEquals(1,service.closes)
    }
    @Test fun currencyAndBudgetChecksUseExactIntegersWithoutOverflow() {
        assertEquals("$0.00", aiMoney(0u))
        assertEquals("$0.000001", aiMoney(1u))
        assertEquals("$5.10", aiMoney(5_100_000u))
        val daily = AiReadiness(true, true, true, 4_000_000u, 5_000_000u)
        assertFalse(aiBudgetExceeded(daily, 1_000_000u))
        assertTrue(aiBudgetExceeded(daily, 1_000_001u))
        assertTrue(aiBudgetExceeded(daily, ULong.MAX_VALUE))
        assertEquals(5_100_001uL, aiParseMoney("5.100001"))
        assertEquals(0uL, aiParseMoney("0"))
        for (invalid in listOf("-1", "1e3", "1.0000001", ".5", "1.", " 5", "18446744073709551615"))
            assertNull(aiParseMoney(invalid))
    }
    @Test fun budgetChangeUsesDisplayedConfigurationFingerprintAndExactMoney() = runBlocking {
        val service = Service()
        val owner = AiServiceController(this, "fixture-private") { _, _ -> service }
        owner.activate(); idle(owner)
        owner.configureDailyBudget("policy-one", 8_500_001uL); idle(owner)
        assertEquals("policy-one" to 8_500_001uL, service.budgetWith)
        assertEquals(8_500_001uL, owner.state.value.configuration!!.dailySoftBudgetMicrousd)
        owner.close()
    }
}
