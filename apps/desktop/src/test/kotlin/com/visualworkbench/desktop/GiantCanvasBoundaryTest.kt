package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

/** Synthetic owner/callback tests are not ComposePanel/GPU retirement proof.
 * No window, native pixel upload, device, provider or screenshot is used here. */
class GiantCanvasBoundaryTest {
    private val binding = WorkflowBinding("project", "document", 1uL, "c".repeat(64))
    private val viewport = GiantViewport(0.0, 0.0, 256.0, 256.0, 1.0, 0.0, 0.0, 1.0)

    @Test fun nestedSceneUsesTheCurrentFocusAndModalPolicyOnce() {
        val route = GiantShortcutRoute<String>(); val received = mutableListOf<String>()
        var focused = true; var modal = false
        route.update { event -> if (focused && !modal) { received += event; true } else false }
        assertTrue(route.dispatch("undo")); assertEquals(listOf("undo"), received)
        modal = true; assertFalse(route.dispatch("delete"))
        modal = false; focused = false; assertFalse(route.dispatch("undo"))
        route.update { received += "replacement:$it"; true }
        assertTrue(route.dispatch("fit")); assertEquals(listOf("undo", "replacement:fit"), received)
        route.seal(); route.update { error("Sealed handler was installed") }
        assertFalse(route.dispatch("delete")); assertEquals(2, received.size)
    }

    @Test fun shortcutReentrancyCannotRedispatchTheSameInput() {
        val route = GiantShortcutRoute<String>(); var calls = 0
        route.update { event -> calls++; assertFalse(route.dispatch(event)); true }
        assertTrue(route.dispatch("undo")); assertEquals(1, calls)
        route.clear(); assertFalse(route.dispatch("undo"))
    }

    @Test fun physicalSurfaceAdmissionUsesActualMonitorScaleAndCeiling() {
        assertTrue(giantSurfaceAdmitted(2048, 2048, 2.0, 2.0))
        assertFalse(giantSurfaceAdmitted(2049, 2048, 2.0, 2.0))
        assertTrue(giantSurfaceAdmitted(8192, 2048, 1.0, 1.0))
        assertFalse(giantSurfaceAdmitted(8193, 1, 1.0, 1.0))
        assertFalse(giantSurfaceAdmitted(4096, 1, 2.00001, 1.0))
        assertTrue(giantSurfaceAdmitted(1000, 2000, 1.5, 2.0))
        for (bad in listOf(0.0, -1.0, Double.NaN, Double.POSITIVE_INFINITY)) {
            assertFalse(giantSurfaceAdmitted(100, 100, bad, 1.0))
            assertFalse(giantSurfaceAdmitted(100, 100, 1.0, bad))
        }
        assertFalse(giantSurfaceAdmitted(0, 100, 1.0, 1.0))
        assertFalse(giantSurfaceAdmitted(100, -1, 1.0, 1.0))
        assertFalse(giantSurfaceAdmitted(Int.MAX_VALUE, Int.MAX_VALUE, Double.MAX_VALUE, 1.0))
    }

    @Test fun rasterAccountingIncludesStrideReplyTextureAndBookkeeping() {
        assertTrue(giantRasterAdmitted(256, 256, 1024, 262144))
        assertTrue(giantRasterAdmitted(255, 256, 1024, 255L * 256 * 4))
        assertTrue(giantRasterAdmitted(1, 1, 4, 4))
        assertFalse(giantRasterAdmitted(256, 256, 1040, 262144))
        assertFalse(giantRasterAdmitted(256, 256, 1023, 262144))
        assertFalse(giantRasterAdmitted(256, 256, 1024, 262143))
        assertFalse(giantRasterAdmitted(257, 1, 1028, 1028))
        assertFalse(giantRasterAdmitted(1, 0, 4, 0))
        assertFalse(giantRasterAdmitted(1, 1, Long.MAX_VALUE, 4))
    }

    @Test fun actualDrawFenceRejectsCopiedStaleGenerationRevisionEpochAndView() {
        val frame = GiantVisibleFrame<Int>(binding, 7L, 4uL, viewport, "a".repeat(64), "b".repeat(64), emptyList())
        val ready = GiantViewStatus(4uL, ready = true)
        fun match(candidate: GiantVisibleFrame<Int>? = frame, bound: WorkflowBinding = binding,
            epoch: Long = 7L, view: GiantViewport? = viewport, state: GiantViewStatus = ready,
            admitted: Boolean = true, current: Boolean = true) =
            giantFrameCurrent(candidate, frame, bound, epoch, view, state, admitted, current)
        assertSame(frame, match())
        assertNull(match(frame.copy()))
        assertNull(match(frame.copy(assetId = "d".repeat(64))))
        assertNull(match(bound = binding.copy(hostSeq = 2uL)))
        assertNull(match(bound = binding.copy(stateHash = "d".repeat(64))))
        assertNull(match(epoch = 8L)); assertNull(match(view = viewport.copy(left = 1.0)))
        assertNull(match(view = null)); assertNull(match(state = ready.copy(generation = 5uL)))
        assertNull(match(state = ready.copy(ready = false)))
        assertNull(match(admitted = false)); assertNull(match(current = false))
    }

    @Test fun graphicsSlotIsClearedBeforeReentrantDetachAndDispose() {
        val events = mutableListOf<String>()
        lateinit var epoch: GiantPanelEpoch<String>
        epoch = GiantPanelEpoch({ events += "cancel" }, {
            assertNull(epoch.current); assertTrue(epoch.retiring)
            assertThrows(IllegalStateException::class.java) { epoch.install("unreserved successor") }
            events += "detach:$it"
        }, { assertNull(epoch.current); events += "dispose:$it" })
        epoch.install("context"); epoch.retire(); epoch.retire()
        assertEquals(listOf("cancel", "detach:context", "dispose:context"), events)
    }

    @Test fun failedGraphicsRetirementCannotRetryOrPretendTheContextWasDisposed() {
        val failure = IllegalStateException("dispose outcome unknown"); var disposed = 0; var detached = 0
        val epoch = GiantPanelEpoch<String>({}, { detached++ }, { disposed++; throw failure })
        epoch.install("context")
        assertSame(failure, assertThrows(IllegalStateException::class.java) { epoch.retire() })
        assertSame(failure, assertThrows(IllegalStateException::class.java) { epoch.retire() })
        assertEquals(1, detached); assertEquals(1, disposed); assertNull(epoch.current)
        assertThrows(IllegalStateException::class.java) { epoch.install("replacement") }
    }

    @Test fun failedDetachCannotReachDisposeOrAnotherEpoch() {
        val failure = IllegalStateException("detach outcome unknown"); var disposed = 0
        val epoch = GiantPanelEpoch<String>({}, { throw failure }, { disposed++ })
        epoch.install("context")
        assertSame(failure, assertThrows(IllegalStateException::class.java) { epoch.retire() })
        assertSame(failure, assertThrows(IllegalStateException::class.java) { epoch.retire() })
        assertEquals(0, disposed)
    }

    @Test fun nativeReleaseDoesNotRepeatADestructorWithAnUnknownOutcome() {
        val failure = IllegalStateException("native close outcome unknown"); var calls = 0
        val failed = GiantReleaseAction { calls++; throw failure }
        assertSame(failure, assertThrows(IllegalStateException::class.java) { failed.close() })
        assertSame(failure, assertThrows(IllegalStateException::class.java) { failed.close() })
        assertEquals(1, calls)
        val successful = GiantReleaseAction { calls++ }; successful.close(); successful.close()
        assertEquals(2, calls)
    }

    @Test fun cancelledConcurrentClosersJoinRetirementBeforeCreditingTheSurface() = runBlocking {
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        val events = mutableListOf<String>(); var bytes = GIANT_DESKTOP_SURFACE_BYTES
        val owner = GiantDesktopRetirement({ events += "seal" }, {
            events += "retire:start"; entered.complete(Unit); release.await()
            events += "dispose"; events += "tiles:release"; events += "native:close"
        }, { bytes = 0uL; events += "surface:credit" })
        val first = launch(start = CoroutineStart.UNDISPATCHED) { owner.close() }
        entered.await()
        val second = launch(start = CoroutineStart.UNDISPATCHED) { owner.close() }
        first.cancel()
        assertTrue(owner.sealed); assertFalse(first.isCompleted); assertFalse(second.isCompleted)
        assertEquals(GIANT_DESKTOP_SURFACE_BYTES, bytes)
        release.complete(Unit)
        withContext(NonCancellable) { first.join(); second.join(); owner.close() }
        assertEquals(0uL, bytes)
        assertEquals(listOf("seal", "retire:start", "dispose", "tiles:release", "native:close", "surface:credit"), events)
    }

    @Test fun failedRetirementNativeCloseOrSurfaceReleaseRetainsTheCreditAndFailure() = runBlocking {
        for (stage in listOf("retire", "native", "surface")) {
            val error = IllegalStateException(stage); var settlements = 0; var credits = 0
            var bytes = GIANT_DESKTOP_SURFACE_BYTES
            val owner = GiantDesktopRetirement({}, {
                settlements++
                if (stage == "retire" || stage == "native") throw error
            }, {
                credits++; if (stage == "surface") throw error
                bytes = 0uL
            })
            assertSame(error, runCatching { owner.close() }.exceptionOrNull())
            assertSame(error, runCatching { owner.close() }.exceptionOrNull())
            assertTrue(owner.sealed); assertEquals(GIANT_DESKTOP_SURFACE_BYTES, bytes)
            assertEquals(1, settlements); assertEquals(if (stage == "surface") 1 else 0, credits)
        }
    }

    @Test fun failedFactoryReservationStillJoinsItsOwnedImage() = runBlocking {
        val image = EmptyImage(); var reservations = 0
        val resources = object : SessionResources {
            override suspend fun reserveForeign(bytes: ULong): SessionResourceCharge {
                reservations++; assertEquals(GIANT_DESKTOP_SURFACE_BYTES, bytes)
                throw GiantImageFailure(GiantImageFailureKind.Memory)
            }
        }
        val error = runCatching { GiantDesktopView.create(this, image, resources, binding, 7L, { _, _ -> true }, {}, {}) }.exceptionOrNull()
        assertTrue(error is GiantImageFailure); assertEquals(1, reservations); assertEquals(1, image.closed)
    }

    @Test fun staleFactoryReleasesSurfaceOnlyAfterNativeImageClose() = runBlocking {
        val events = mutableListOf<String>(); val image = EmptyImage { events += "native:close" }
        val resources = object : SessionResources {
            override suspend fun reserveForeign(bytes: ULong): SessionResourceCharge {
                events += "surface:reserve"
                return object : SessionResourceCharge { override fun close() { events += "surface:credit" } }
            }
        }
        assertTrue(runCatching { GiantDesktopView.create(this, image, resources, binding, 7L, { _, _ -> false }, {}, {}) }.isFailure)
        assertEquals(listOf("surface:reserve", "native:close", "surface:credit"), events)
    }

    @Test fun failedFactoryCleanupExposesTheRetainedOwnerForRootQuarantine() = runBlocking {
        val error = IllegalStateException("native close failed"); var credits = 0
        val image = EmptyImage { throw error }
        val resources = object : SessionResources {
            override suspend fun reserveForeign(bytes: ULong): SessionResourceCharge =
                object : SessionResourceCharge { override fun close() { credits++ } }
        }
        val failure = runCatching { GiantDesktopView.create(this, image, resources, binding, 7L, { _, _ -> false }, {}, {}) }.exceptionOrNull()
        assertTrue(failure is GiantDesktopCreationFailure)
        val retained = (failure as GiantDesktopCreationFailure).owner
        assertFalse(retained.surfaceReserved); assertFalse(retained.status.ready)
        assertSame(error, runCatching { retained.close() }.exceptionOrNull())
        assertEquals(1, image.closed); assertEquals(0, credits)
    }

    @Test fun cancelledFactoryScopeCannotAbandonAReservationOrImage() = runBlocking {
        val parent = Job().apply { cancel() }; val image = EmptyImage(); var credits = 0
        val resources = object : SessionResources {
            override suspend fun reserveForeign(bytes: ULong): SessionResourceCharge =
                object : SessionResourceCharge { override fun close() { credits++ } }
        }
        assertTrue(runCatching { GiantDesktopView.create(CoroutineScope(coroutineContext + parent), image,
            resources, binding, 7L, { _, _ -> true }, {}, {}) }.exceptionOrNull() is CancellationException)
        assertEquals(1, image.closed); assertEquals(1, credits)
    }

    private class EmptyImage(private val onClose: () -> Unit = {}) : GiantImage {
        var closed = 0
        override suspend fun viewport(view: GiantViewport): GiantViewportPlan = error("No viewport admission expected")
        override suspend fun tile(address: GiantTileAddress, foreign: SessionResourceCharge): GiantTilePixels = error("No pixels expected")
        override suspend fun close() { closed++; onClose() }
    }
}
