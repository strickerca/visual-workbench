package com.visualworkbench.android.remote

import android.graphics.SurfaceTexture
import android.os.Handler
import android.view.Surface
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/** Synthetic platform/lifecycle fixtures. Fake engines never decode video and
 * do not establish codec capability, device latency or physical acceptance. */
@RunWith(AndroidJUnit4::class)
class RemoteDecoderInstrumentedTest {
    private fun config() = DecoderConfig(DecoderScope(1uL, "capture", 1uL, "target", 1u),
        1uL, 100, 100, 99, 99, byteArrayOf(0, 1), byteArrayOf(0, 2), byteArrayOf(0, 3))
    private fun frame(pts: Long = 1, ticket: ULong = 1uL, id: ULong = 1uL, now: Long = 0,
        idr: Boolean = true) = DecoderFrame(ticket, config().scope, 1uL, id, pts, byteArrayOf(1), now, idr)

    @Test fun policy_rejects_oversize_config_odd_coded_geometry_and_unbounded_padding() {
        val config = config(); assertTrue(validDecoderConfig(config))
        assertFalse(validDecoderConfig(config.copy(codedWidth = 4098)))
        assertFalse(validDecoderConfig(config.copy(codedWidth = 99)))
        assertFalse(validDecoderConfig(config.copy(visibleWidth = 98)))
        assertFalse(validDecoderConfig(config.copy(vps = ByteArray(MAX_REMOTE_CONFIG))))
    }
    @Test fun first_frame_requires_idr_and_scope_generation_must_match() {
        val ledger = DecoderTickets(config())
        assertEquals("IdrRequired", ledger.admit(frame(idr = false), 0))
        assertEquals("StaleScope", ledger.admit(frame().copy(generation = 2uL), 0))
        assertEquals("StaleScope", ledger.admit(frame().copy(scope = config().scope.copy(sourceGeneration = 2uL)), 0))
        assertNull(ledger.admit(frame(), 0))
        assertEquals("QueueLoss", ledger.admit(frame(2, 2uL, 2uL), 0))
    }
    @Test fun pts_release_render_callback_and_enqueue_times_remain_distinct() {
        val ledger = DecoderTickets(config()); ledger.admit(frame(now = 10), 10)
        assertNull(ledger.rendered(1, 20, 30)) // release has not happened
        assertFalse(ledger.released(2, 15, 18))
        assertTrue(ledger.released(1, 15, 18))
        assertNull(ledger.rendered(2, 20, 30))
        assertNull(ledger.rendered(1, 20, 19))
        val rendered = ledger.rendered(1, 20, 30)
        assertEquals(1uL, rendered?.ticket)
        assertEquals(15L, ledger.releaseRequestedNanos)
        assertEquals(18L, ledger.outputReleaseReturnedNanos)
        assertNull(ledger.rendered(1, 20, 30))
    }
    @Test fun stale_pts_frame_ticket_and_expired_input_refuse() {
        val ledger = DecoderTickets(config()); ledger.admit(frame(), 0)
        ledger.released(1, 1, 2); ledger.rendered(1, 3, 4)
        assertEquals("InvalidFrame", ledger.admit(frame(1, 2uL, 2uL, 5), 5))
        assertEquals("InvalidFrame", ledger.admit(frame(2, 1uL, 2uL, 5), 5))
        assertEquals("InvalidFrame", ledger.admit(frame(2, 2uL, 1uL, 5), 5))
        assertEquals("InvalidFrame", ledger.admit(frame(2, 2uL, 2uL), FRAME_DEADLINE_NANOS))
        assertNull(ledger.admit(frame(2, 2uL, 2uL, 5, false), 5))
    }
    @Test fun input_bytes_are_copied_and_au_size_is_bounded() {
        val ledger = DecoderTickets(config()); val frame = frame(); ledger.admit(frame, 0)
        frame.annexB[0] = 9; assertEquals(1, ledger.outstanding?.annexB?.single()?.toInt())
        val other = DecoderTickets(config())
        assertEquals("InvalidFrame", other.admit(frame().copy(annexB = ByteArray(MAX_REMOTE_AU + 1)), 0))
    }
    @Test fun borrowed_surface_survives_actual_owned_wrapper_retirement() = runBlocking {
        val texture = SurfaceTexture(false); val surface = Surface(texture)
        val engine = FakeEngine()
        var decoder: RemoteHardwareDecoder? = null
        var primaryFailure: Throwable? = null
        try {
            decoder = started(surface, texture, engine)
            assertTrue(engine.entered.await(2, TimeUnit.SECONDS))
            assertTrue(surface.isValid)
            assertEquals(DecoderRetirement.Retired, decoder.close())
            assertTrue(engine.released.get()); assertTrue(surface.isValid)
            assertFalse(RemoteHardwareDecoder.hasPendingOwner())
        } catch (problem: Throwable) { primaryFailure = problem; throw problem }
        finally {
            retireFixture(listOf(decoder), surface, texture, primaryFailure)
        }
    }
    @Test fun blocked_configuration_retains_permit_and_surface_until_actual_join() = runBlocking {
        val texture = SurfaceTexture(false); val surface = Surface(texture)
        val gate = CountDownLatch(1); val engine = FakeEngine(gate)
        var decoder: RemoteHardwareDecoder? = null
        var primaryFailure: Throwable? = null
        try {
            decoder = started(surface, texture, engine)
            assertTrue(engine.entered.await(2, TimeUnit.SECONDS))
            assertTrue(decoder.close() is DecoderRetirement.Pending)
            assertFalse(engine.released.get()); assertTrue(RemoteHardwareDecoder.hasPendingOwner())
            assertTrue(RemoteHardwareDecoder.openOwned(surface, texture, config(), { _, _, _, _ -> }) { FakeEngine() } is DecoderOpenResult.Refused)
            val original = AssertionError("fixture body assertion")
            retireFixture(listOf(decoder), surface, texture, original)
            val retained = checkNotNull(PendingFixtures.owners[decoder.ownerId])
            assertSame(surface, retained.surface); assertSame(texture, retained.texture)
            assertSame(decoder, retained.decoders.single())
            assertEquals(4, retained.observations.size)
            assertTrue(retained.observations.all { it is DecoderRetirement.Pending })
            assertSame(retained.cleanupFailure, original.suppressed.single())
            assertTrue(surface.isValid); assertTrue(RemoteHardwareDecoder.hasPendingOwner())
            gate.countDown(); assertEquals(DecoderRetirement.Retired, decoder.close())
            assertTrue(engine.released.get()); assertTrue(surface.isValid)
        } catch (problem: Throwable) { primaryFailure = problem; throw problem }
        finally { gate.countDown(); retireFixture(listOf(decoder), surface, texture, primaryFailure) }
    }
    @Test fun failed_release_stays_owned_and_explicit_close_retries_same_engine() = runBlocking {
        val texture = SurfaceTexture(false); val surface = Surface(texture)
        val engine = FakeEngine(failRelease = AtomicBoolean(true))
        var decoder: RemoteHardwareDecoder? = null
        var primaryFailure: Throwable? = null
        try {
            decoder = started(surface, texture, engine); assertTrue(engine.entered.await(2, TimeUnit.SECONDS))
            assertTrue(decoder.close() is DecoderRetirement.Pending)
            assertTrue(RemoteHardwareDecoder.hasPendingOwner()); assertFalse(engine.released.get())
            engine.failRelease.set(false)
            assertEquals(DecoderRetirement.Retired, decoder.close())
            assertTrue(engine.released.get())
        } catch (problem: Throwable) { primaryFailure = problem; throw problem }
        finally { engine.failRelease.set(false); retireFixture(listOf(decoder), surface, texture, primaryFailure) }
    }
    @Test fun same_config_recovery_has_fresh_owner_and_rejects_delta_start() = runBlocking {
        val texture = SurfaceTexture(false); val surface = Surface(texture)
        var first: RemoteHardwareDecoder? = null; var second: RemoteHardwareDecoder? = null
        var primaryFailure: Throwable? = null
        try {
            val engine = FakeEngine(); first = started(surface, texture, engine)
            assertTrue(engine.entered.await(2, TimeUnit.SECONDS))
            awaitReady(first); val firstId = first.ownerId
            first.requestRecovery("QueueLoss"); assertEquals(DecoderRetirement.Retired, first.close())
            val next = FakeEngine(); second = started(surface, texture, next)
            assertTrue(next.entered.await(2, TimeUnit.SECONDS)); awaitReady(second)
            assertNotEquals(firstId, second.ownerId)
            assertTrue(second.queue(frame(now = System.nanoTime(), idr = false)) is DecoderQueueResult.RecoveryRequired)
            assertEquals(DecoderRetirement.Retired, second.close())
        } catch (problem: Throwable) { primaryFailure = problem; throw problem }
        finally { retireFixture(listOf(first, second), surface, texture, primaryFailure) }
    }
    @Test fun actual_callback_thread_is_retained_until_it_joins_and_old_callback_cannot_ack() = runBlocking {
        val texture = SurfaceTexture(false); val surface = Surface(texture)
        val gate = CountDownLatch(1); val engine = FakeEngine(renderFrames = true, renderGate = gate)
        val userCallbacks = CountDownLatch(1); var decoder: RemoteHardwareDecoder? = null
        var primaryFailure: Throwable? = null
        try {
            decoder = when (val opened = RemoteHardwareDecoder.openOwned(surface, texture, config(),
                { _, _, _, _ -> userCallbacks.countDown() }) { engine }) {
                is DecoderOpenResult.Started -> opened.decoder
                is DecoderOpenResult.Refused -> throw AssertionError(opened.reason)
            }
            assertTrue(engine.entered.await(2, TimeUnit.SECONDS)); awaitReady(decoder)
            assertEquals(DecoderQueueResult.Accepted, decoder.queue(frame(now = System.nanoTime())))
            assertTrue(engine.renderEntered.await(2, TimeUnit.SECONDS))
            assertTrue(decoder.close() is DecoderRetirement.Pending)
            assertTrue(engine.released.get()); assertTrue(RemoteHardwareDecoder.hasPendingOwner())
            assertEquals(1L, userCallbacks.count)
            gate.countDown(); assertEquals(DecoderRetirement.Retired, decoder.close())
            assertEquals(1L, userCallbacks.count)
        } catch (problem: Throwable) { primaryFailure = problem; throw problem }
        finally { gate.countDown(); retireFixture(listOf(decoder), surface, texture, primaryFailure) }
    }
    @Test fun successful_surface_callback_returns_exact_ticket_without_editor_effect_claim() = runBlocking {
        val texture = SurfaceTexture(false); val surface = Surface(texture)
        val engine = FakeEngine(renderFrames = true); val callback = CountDownLatch(1)
        var returnedTicket = 0uL; var returnedPts = 0L; var ordered = false
        var decoder: RemoteHardwareDecoder? = null
        var primaryFailure: Throwable? = null
        try {
            decoder = when (val opened = RemoteHardwareDecoder.openOwned(surface, texture, config(),
                { ticket, pts, rendered, arrived ->
                    returnedTicket = ticket; returnedPts = pts; ordered = arrived >= rendered; callback.countDown()
                }) { engine }) {
                is DecoderOpenResult.Started -> opened.decoder
                is DecoderOpenResult.Refused -> throw AssertionError(opened.reason)
            }
            assertTrue(engine.entered.await(2, TimeUnit.SECONDS)); awaitReady(decoder)
            assertEquals(DecoderQueueResult.Accepted, decoder.queue(frame(now = System.nanoTime())))
            assertTrue(callback.await(2, TimeUnit.SECONDS))
            assertEquals(1uL, returnedTicket); assertEquals(1L, returnedPts); assertTrue(ordered)
            val timing = checkNotNull(decoder.lastTiming.value)
            assertEquals(decoder.ownerId, timing.ownerId)
            assertTrue(timing.outputReleaseRequestedLocalNanos >= timing.enqueuedLocalNanos)
            assertTrue(timing.outputReleaseReturnedLocalNanos >= timing.outputReleaseRequestedLocalNanos)
            assertTrue(timing.renderedLocalNanos >= timing.outputReleaseRequestedLocalNanos)
            assertTrue(timing.callbackLocalNanos >= timing.renderedLocalNanos)
            assertEquals(DecoderRetirement.Retired, decoder.close())
            assertNull(decoder.lastTiming.value)
        } catch (problem: Throwable) { primaryFailure = problem; throw problem }
        finally { retireFixture(listOf(decoder), surface, texture, primaryFailure) }
    }
    @Test fun blocked_surface_release_retains_its_io_worker_and_permit_until_join() = runBlocking {
        val texture = SurfaceTexture(false); val surface = Surface(texture)
        val gate = CountDownLatch(1); val releaseEntered = CountDownLatch(1)
        val engine = FakeEngine(); var decoder: RemoteHardwareDecoder? = null
        var primaryFailure: Throwable? = null
        try {
            decoder = when (val opened = RemoteHardwareDecoder.openOwned(surface, texture, config(),
                { _, _, _, _ -> }, releaseSurface = { owned ->
                    releaseEntered.countDown(); gate.await(); owned.release()
                }) { engine }) {
                is DecoderOpenResult.Started -> opened.decoder
                is DecoderOpenResult.Refused -> throw AssertionError(opened.reason)
            }
            assertTrue(engine.entered.await(2, TimeUnit.SECONDS)); awaitReady(decoder)
            assertTrue(decoder.close() is DecoderRetirement.Pending)
            assertTrue(releaseEntered.await(2, TimeUnit.SECONDS))
            assertTrue(engine.released.get()); assertTrue(RemoteHardwareDecoder.hasPendingOwner())
            assertTrue(surface.isValid)
            gate.countDown(); assertEquals(DecoderRetirement.Retired, decoder.close())
            assertFalse(RemoteHardwareDecoder.hasPendingOwner()); assertTrue(surface.isValid)
        } catch (problem: Throwable) { primaryFailure = problem; throw problem }
        finally { gate.countDown(); retireFixture(listOf(decoder), surface, texture, primaryFailure) }
    }

    private data class FixtureFailureOwner(
        val decoders: List<RemoteHardwareDecoder>, val surface: Surface,
        val texture: SurfaceTexture, val observations: List<DecoderRetirement>,
        val cleanupFailure: Throwable?,
    )
    private object PendingFixtures {
        // Strong explicit fixture ownership, including borrowed consumer objects.
        // Pending is a failed cleanup receipt; nothing is labelled disposed.
        val owners = ConcurrentHashMap<String, FixtureFailureOwner>()
    }
    private suspend fun retireFixture(
        decoders: List<RemoteHardwareDecoder?>, surface: Surface, texture: SurfaceTexture,
        primaryFailure: Throwable? = null,
    ) = withContext(NonCancellable + Dispatchers.IO) {
        val owned = decoders.filterNotNull().distinctBy { it.ownerId }
        val key = owned.lastOrNull()?.ownerId ?: "fixture-${System.identityHashCode(surface)}"
        var remaining = owned
        val observations = mutableListOf<DecoderRetirement>()
        PendingFixtures.owners[key] = FixtureFailureOwner(owned, surface, texture, emptyList(), null)
        // Four observations of the SAME owners; no restart, new decoder,
        // shortened native join or replacement admission. Each close retains
        // its original500 ms observation budget. Existing test deadlines stay.
        for (attempt in 0 until 4) {
            val pending = mutableListOf<RemoteHardwareDecoder>()
            for (decoder in remaining) {
                val receipt = try { decoder.close() }
                    catch (failure: Exception) {
                        DecoderRetirement.Pending("FixtureClose:${failure.javaClass.simpleName}")
                    }
                observations += receipt
                if (receipt != DecoderRetirement.Retired) pending += decoder
            }
            remaining = pending
            if (remaining.isEmpty()) break
        }
        if (remaining.isNotEmpty()) {
            val failure = AssertionError("Fixture cleanup Pending; decoder/Surface/texture retained by failure owner $key")
            PendingFixtures.owners[key] = FixtureFailureOwner(remaining, surface, texture, observations.toList(), failure)
            if (primaryFailure != null) primaryFailure.addSuppressed(failure) else throw failure
            return@withContext
        }
        try {
            // Borrowed fixture resources retire only after every actual decoder
            // and native/IO/callback owner reported Retired.
            surface.release()
            texture.release()
            PendingFixtures.owners.remove(key)
        } catch (failure: Throwable) {
            PendingFixtures.owners[key] = FixtureFailureOwner(owned, surface, texture, observations.toList(), failure)
            if (primaryFailure != null) primaryFailure.addSuppressed(failure) else throw failure
        }
    }

    private fun started(surface: Surface, consumer: Any, engine: FakeEngine): RemoteHardwareDecoder =
        when (val result = RemoteHardwareDecoder.openOwned(surface, consumer, config(), { _, _, _, _ -> }) { engine }) {
            is DecoderOpenResult.Started -> result.decoder
            is DecoderOpenResult.Refused -> throw AssertionError(result.reason)
        }
    private fun awaitReady(decoder: RemoteHardwareDecoder) {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(2)
        while (decoder.state.value == DecoderStatus.Starting && System.nanoTime() < deadline) Thread.sleep(2)
        assertTrue(decoder.state.value is DecoderStatus.Ready)
    }
    private class FakeEngine(private val startGate: CountDownLatch? = null,
        val failRelease: AtomicBoolean = AtomicBoolean(false),
        private val renderFrames: Boolean = false, private val renderGate: CountDownLatch? = null) : DecoderEngine {
        val entered = CountDownLatch(1); val released = AtomicBoolean(false)
        val renderEntered = CountDownLatch(1)
        private var callback: ((Long, Long, Long) -> Unit)? = null
        private var handler: Handler? = null
        private var pts: Long? = null
        private var outputGiven = false
        override fun start(config: DecoderConfig, surface: Surface, callbacks: Handler,
            onRendered: (Long, Long, Long) -> Unit): DecoderAdmission {
            this.callback = onRendered; this.handler = callbacks
            entered.countDown(); startGate?.await()
            return DecoderAdmission("fixture", true, false, true, true, true, emptyList())
        }
        override fun queue(bytes: ByteArray, ptsUs: Long): Boolean {
            if (!renderFrames) return false
            pts = ptsUs; return true
        }
        override fun output(): DecoderOutput {
            val current = pts
            if (current == null || outputGiven) return DecoderOutput.None
            outputGiven = true; return DecoderOutput.Buffer(0, current, 0)
        }
        override fun outputDimensions(): Pair<Int, Int> = Pair(100, 100)
        override fun discardOutput(index: Int): Unit = Unit
        override fun renderOutput(index: Int, requestedLocalNanos: Long) {
            checkNotNull(handler).post {
                renderEntered.countDown(); renderGate?.await()
                // Fake codec echoes the actual requested Surface timestamp;
                // media PTS remains independent. No host-derived clock offset.
                checkNotNull(callback).invoke(checkNotNull(pts), requestedLocalNanos, System.nanoTime())
            }
        }
        override fun stop(): Unit = Unit
        override fun release() { check(!failRelease.get()) { "fixture release pending" }; released.set(true) }
    }
}
