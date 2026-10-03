package com.visualworkbench.desktop

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import org.junit.Assert.*
import org.junit.Test

class ConnectionAssistanceTest {
    @Test fun constructingAndSelectingAToolNeverRunsInventoryOrStartsRecovery() = runBlocking {
        val port = AssistFixture(); val controller = ConnectionAssistanceController(this, port)
        try {
            controller.selectTool("C:\\approved\\adb.exe"); yield()
            assertEquals(0, port.inventories); assertTrue(port.selections.isEmpty()); assertEquals(0, port.metricReads)
            controller.inspectDevices(); awaitIdle(controller)
            assertEquals(1, port.inventories); assertNull(controller.state.value.selectedSerial)
            assertTrue(port.selections.isEmpty())
        } finally { controller.close() }
    }
    @Test fun watchRequiresAnOwnerSelectedInventoryDeviceAndImmutableExplicitPorts() = runBlocking {
        val port = AssistFixture(); val controller = selected(this, port)
        try {
            controller.selectDevice("not-in-inventory"); controller.startReverse("47191", "47192")
            assertTrue(port.selections.isEmpty())
            controller.selectDevice("test-selected"); controller.startReverse("47191", "47192"); awaitIdle(controller)
            assertEquals(ReverseSelection("C:\\approved\\adb.exe", "test-selected", 47191u, 47192u), port.selections.single())
            controller.selectTool("C:\\another\\adb.exe"); controller.selectDevice("test-other")
            assertEquals("C:\\approved\\adb.exe", controller.state.value.adbPath)
            assertEquals("test-selected", controller.state.value.selectedSerial)
            assertEquals(1, port.inventories)
        } finally { controller.close() }
    }
    @Test fun invalidPortsNeverReachTheNativeAdapter() = runBlocking {
        val port = AssistFixture(); val controller = selected(this, port); controller.selectDevice("test-selected")
        try {
            for (portText in listOf("", "0", "80", "65536", "47191;remove-all", " 47191")) controller.startReverse(portText, "47192")
            assertTrue(port.selections.isEmpty())
        } finally { controller.close() }
    }
    @Test fun stopClosesOnlyTheOwnedWatchAndStatesThatMappingsRemain() = runBlocking {
        val port = AssistFixture(); val controller = selected(this, port); controller.selectDevice("test-selected")
        try {
            controller.startReverse("47191", "47192"); awaitIdle(controller)
            assertTrue(controller.state.value.watching)
            controller.stopReverse(); awaitIdle(controller)
            assertEquals(1, port.reverse.closed); assertFalse(controller.state.value.watching)
            assertTrue(controller.state.value.message!!.contains("left in place"))
            assertTrue(controller.state.value.reverse!!.mappingMayRemain)
        } finally { controller.close() }
        assertEquals(1, port.reverse.closed); assertEquals(listOf("watch-close", "port-close"), port.order)
    }
    @Test fun lateWatchAfterCancellationIsClosedBeforeAdapterShutdown() = runBlocking {
        val port = AssistFixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        port.beforeWatch = { entered.complete(Unit); withContext(NonCancellable) { release.await() } }
        val controller = selected(this, port); controller.selectDevice("test-selected"); controller.startReverse("47191", "47192")
        entered.await(); val closing = launch { controller.close() }; yield()
        assertFalse(closing.isCompleted); release.complete(Unit); closing.join()
        assertEquals(1, port.reverse.closed); assertEquals(listOf("watch-close", "port-close"), port.order)
        assertFalse(controller.state.value.watching)
    }
    @Test fun routeInspectionDoesNotApplyAndAResultOffersTheExactBoundInverse() = runBlocking {
        val port = AssistFixture(); val controller = ConnectionAssistanceController(this, port)
        try {
            controller.prepareMetric(7u, MetricFamily.Ipv4); awaitIdle(controller)
            assertEquals(1, port.metricReads); assertEquals(0, port.plan.applied)
            assertEquals(5u, controller.state.value.metric!!.oldMetric); assertEquals(500u, controller.state.value.metric!!.newMetric)
            controller.applyMetric(); awaitIdle(controller)
            assertEquals(1, port.plan.applied); assertEquals(1, port.plan.closed)
            assertNull(controller.state.value.metric)
            assertEquals(port.inverse.details, controller.state.value.reverts.single())
            assertEquals(MetricKind.Revert, controller.state.value.reverts.single().kind)
            assertEquals(0, port.inverse.applied)
        } finally { controller.close() }
        assertEquals(1, port.inverse.closed); assertEquals(0, port.inverse.applied)
    }
    @Test fun staleOrUncertainMetricActionsAreConsumedWithoutInventingARevert() = runBlocking {
        for (message in listOf("Routes changed", "Metric outcome unknown")) {
            val port = AssistFixture(); port.plan.failure = AssistanceFailure(message)
            val controller = ConnectionAssistanceController(this, port)
            try {
                controller.prepareMetric(7u, MetricFamily.Ipv4); awaitIdle(controller)
                controller.applyMetric(); awaitIdle(controller)
                assertNull(controller.state.value.metric); assertEquals(message, controller.state.value.message)
                controller.applyMetric(); assertEquals(1, port.plan.applied); assertEquals(1, port.plan.closed)
            } finally { controller.close() }
        }
    }
    @Test fun declinedRevertRetainsItsExactOriginalValuesAndCanBeRetriedExplicitly() = runBlocking {
        for (message in listOf("Administrator approval declined", "No worker admission", "Cancelled before setter")) {
            val port = AssistFixture(); val controller = ConnectionAssistanceController(this, port)
            val redo = MetricFixture(port.plan.details); port.inverse.inverse = redo
            try {
                controller.prepareMetric(7u, MetricFamily.Ipv4); awaitIdle(controller); controller.applyMetric(); awaitIdle(controller)
                port.inverse.failure = AssistanceFailure(message, retryableMetric = true)
                controller.revertMetric(7u, MetricFamily.Ipv4); awaitIdle(controller)
                assertEquals(1, port.inverse.applied); assertEquals(0, port.inverse.closed)
                assertEquals(listOf(port.inverse.details), controller.state.value.reverts)
                assertTrue(controller.state.value.recoveries.isEmpty())
                port.inverse.failure = null
                controller.revertMetric(7u, MetricFamily.Ipv4); awaitIdle(controller)
                assertEquals(2, port.inverse.applied); assertEquals(1, port.inverse.closed)
                assertTrue(controller.state.value.reverts.isEmpty()); assertTrue(controller.state.value.recoveries.isEmpty())
                assertEquals(1, redo.closed); assertEquals(0, redo.applied)
            } finally { controller.close() }
            assertEquals(1, port.inverse.closed)
        }
    }
    @Test fun uncertainOrStaleRevertKeepsOriginalRecoveryValuesAfterConsumingTheAction() = runBlocking {
        for (message in listOf("Routes changed", "Metric outcome unknown")) {
            val port = AssistFixture(); val controller = ConnectionAssistanceController(this, port)
            try {
                controller.prepareMetric(7u, MetricFamily.Ipv4); awaitIdle(controller); controller.applyMetric(); awaitIdle(controller)
                port.inverse.failure = AssistanceFailure(message)
                controller.revertMetric(7u, MetricFamily.Ipv4); awaitIdle(controller)
                assertTrue(controller.state.value.reverts.isEmpty()); assertEquals(1, port.inverse.closed)
                assertEquals(MetricRecovery(MetricFamily.Ipv4, 7u, true, 5u), controller.state.value.recoveries.single())
                controller.revertMetric(7u, MetricFamily.Ipv4); assertEquals(1, port.inverse.applied)
            } finally { controller.close() }
        }
    }
    @Test fun aLaterSuccessfulFixAndItsRevertCannotEraseEarlierUncertainOriginals() = runBlocking {
        val port = AssistFixture(); val controller = ConnectionAssistanceController(this, port)
        val later = MetricFixture(MetricDetails(MetricKind.Fix, MetricFamily.Ipv4, 7u, false, 500u, false, 1050u))
        val inverse = MetricFixture(MetricDetails(MetricKind.Revert, MetricFamily.Ipv4, 7u, false, 1050u, false, 500u))
        val redo = MetricFixture(later.details); inverse.inverse = redo; later.inverse = inverse
        try {
            port.plan.failure = AssistanceFailure("Metric outcome unknown")
            controller.prepareMetric(7u, MetricFamily.Ipv4); awaitIdle(controller); controller.applyMetric(); awaitIdle(controller)
            val original = MetricRecovery(MetricFamily.Ipv4, 7u, true, 5u)
            assertEquals(listOf(original), controller.state.value.recoveries)
            port.nextPlan = later
            controller.prepareMetric(7u, MetricFamily.Ipv4); awaitIdle(controller); controller.applyMetric(); awaitIdle(controller)
            assertEquals(listOf(original), controller.state.value.recoveries)
            controller.revertMetric(7u, MetricFamily.Ipv4); awaitIdle(controller)
            assertEquals(listOf(original), controller.state.value.recoveries)
            controller.resolveRecovery(7u, MetricFamily.Ipv4)
            assertTrue(controller.state.value.recoveries.isEmpty())
            assertEquals(1, later.applied); assertEquals(1, inverse.applied); assertEquals(0, redo.applied)
        } finally { controller.close() }
    }
    @Test fun inspectingAnotherFamilyRetainsOlderRevertsAndOnlyTheNewestCanRun() = runBlocking {
        val port = AssistFixture(); val controller = ConnectionAssistanceController(this, port)
        val inverse6 = MetricFixture(port.inverse.details.copy(family = MetricFamily.Ipv6))
        val redo6 = MetricFixture(port.plan.details.copy(family = MetricFamily.Ipv6)); inverse6.inverse = redo6
        val plan6 = MetricFixture(port.plan.details.copy(family = MetricFamily.Ipv6)).also { it.inverse = inverse6 }
        try {
            controller.prepareMetric(7u, MetricFamily.Ipv4); awaitIdle(controller); controller.applyMetric(); awaitIdle(controller)
            port.nextPlan = plan6
            controller.prepareMetric(7u, MetricFamily.Ipv6); awaitIdle(controller); controller.applyMetric(); awaitIdle(controller)
            assertEquals(2, controller.state.value.reverts.size); assertEquals(0, port.inverse.closed)
            controller.revertMetric(7u, MetricFamily.Ipv4); assertEquals(0, port.inverse.applied)
            controller.revertMetric(7u, MetricFamily.Ipv6); awaitIdle(controller)
            assertEquals(1, inverse6.applied); assertEquals(1, inverse6.closed); assertEquals(1, redo6.closed)
            assertEquals(listOf(port.inverse.details), controller.state.value.reverts)
        } finally { controller.close() }
        assertEquals(1, port.inverse.closed)
    }
    @Test fun lateMetricProposalIsDestroyedWhenItsRequestIsCancelled() = runBlocking {
        val port = AssistFixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        port.beforeMetric = { entered.complete(Unit); withContext(NonCancellable) { release.await() } }
        val controller = ConnectionAssistanceController(this, port)
        try {
            controller.prepareMetric(7u, MetricFamily.Ipv4); entered.await(); controller.cancelWork(); release.complete(Unit); awaitIdle(controller)
            assertEquals(1, port.plan.closed); assertNull(controller.state.value.metric)
        } finally { controller.close() }
    }
    @Test fun lateAcceptedMetricReceiptIsSettledAndReleasedWithoutAutomaticRollback() = runBlocking {
        val port = AssistFixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        port.plan.beforeApply = { entered.complete(Unit); withContext(NonCancellable) { release.await() } }
        val controller = ConnectionAssistanceController(this, port)
        controller.prepareMetric(7u, MetricFamily.Ipv4); awaitIdle(controller); controller.applyMetric(); entered.await()
        val closing = launch { controller.close() }; yield(); assertFalse(closing.isCompleted)
        release.complete(Unit); closing.join()
        assertEquals(1, port.plan.applied); assertEquals(1, port.plan.closed); assertEquals(1, port.inverse.closed)
        assertEquals(0, port.inverse.applied); assertEquals("port-close", port.order.last())
    }
    @Test fun alreadyCancelledParentCannotStrandBusyOrAnOwnedProposal() = runBlocking {
        val owner = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val port = AssistFixture(); val controller = ConnectionAssistanceController(owner, port)
        controller.prepareMetric(7u, MetricFamily.Ipv4); awaitIdle(controller)
        owner.cancel(); controller.applyMetric(); yield()
        controller.close(); assertEquals(1, port.plan.closed)
        assertEquals(0, port.inverse.applied)
    }
    @Test fun statusCopyDistinguishesConflictFromAvailabilityAndNeverClaimsTimingProof() {
        val snapshot = ReverseSnapshot(ReverseState.ContestedMapping, 0u, 12u, false, 47191u, 47192u)
        assertTrue(reverseLabel(snapshot).contains("nothing was rebound"))
        assertTrue(reverseLabel(snapshot.copy(state = ReverseState.ServerUnavailable)).contains("not started or reset"))
        assertTrue(reverseLabel(snapshot.copy(state = ReverseState.ExistingMapping)).contains("unknown"))
        assertTrue(reverseLabel(snapshot.copy(state = ReverseState.Stopped)).contains("preserved"))
    }
}

private suspend fun selected(scope: CoroutineScope, port: AssistFixture): ConnectionAssistanceController = ConnectionAssistanceController(scope, port).also {
    it.selectTool("C:\\approved\\adb.exe"); it.inspectDevices(); awaitIdle(it)
}
private suspend fun awaitIdle(controller: ConnectionAssistanceController) { withTimeout(2000) { controller.state.first { !it.busy } } }
private class AssistFixture : ConnectionAssistancePort {
    val order = mutableListOf<String>(); var inventories = 0; var metricReads = 0
    val selections = mutableListOf<ReverseSelection>()
    var beforeWatch: suspend () -> Unit = {}; var beforeMetric: suspend () -> Unit = {}
    val reverse = ReverseFixture(order)
    val inverse = MetricFixture(MetricDetails(MetricKind.Revert, MetricFamily.Ipv4, 7u, false, 500u, true, 5u))
    val plan = MetricFixture(MetricDetails(MetricKind.Fix, MetricFamily.Ipv4, 7u, true, 5u, false, 500u)).also { it.inverse = inverse }
    var nextPlan: MetricLease? = null
    override suspend fun devices(path: String): List<AssistDevice> { inventories++; return listOf(AssistDevice("test-selected", AssistDeviceState.Available), AssistDevice("test-other", AssistDeviceState.Available)) }
    override suspend fun watch(selection: ReverseSelection): ReverseLease { selections += selection; beforeWatch(); return reverse }
    override suspend fun metric(interfaceIndex: UInt, family: MetricFamily): MetricLease { metricReads++; beforeMetric(); return nextPlan ?: plan }
    override suspend fun close() { order += "port-close" }
}
private class ReverseFixture(private val order: MutableList<String>) : ReverseLease {
    var closed = 0
    override fun snapshot() = ReverseSnapshot(ReverseState.MappingCreated, 1u, 20u, true, 47191u, 47192u)
    override suspend fun close() { closed++; order += "watch-close" }
}
private class MetricFixture(override val details: MetricDetails) : MetricLease {
    var closed = 0; var applied = 0; var inverse: MetricLease? = null; var failure: Exception? = null
    var beforeApply: suspend () -> Unit = {}
    override suspend fun apply(): MetricLease { applied++; beforeApply(); failure?.let { throw it }; return checkNotNull(inverse) }
    override fun close() { closed++ }
}
