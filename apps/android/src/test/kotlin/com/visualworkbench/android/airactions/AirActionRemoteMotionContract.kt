package com.visualworkbench.android.airactions

import com.visualworkbench.shared.AirActionBinding
import com.visualworkbench.shared.AirActionBindings
import com.visualworkbench.shared.AirActionCommand
import com.visualworkbench.shared.AirActionContext
import com.visualworkbench.shared.AirActionGesture
import com.visualworkbench.shared.AirActionKeyEvent
import com.visualworkbench.shared.AirActionKeyPhase
import com.visualworkbench.shared.AirActionRouter
import com.visualworkbench.shared.CommandDispatcher
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel

/** Real router and production caller observer. Native readiness/covering ACK is
 * a fixture admission flag; this is not a native ACK or Android/device test. */
internal object AirActionRemoteMotionContract {
    fun scenarios(): Map<String, () -> Unit> = linkedMapOf(
        "unhooked negative control executes stale Undo after covering ACK" to ::negativeControl,
        "hook retires 1001/1002 Swipe before 1010/1040 stroke and 1100 ACK" to ::staleOffer,
        "contact retires offers already queued in serial dispatcher" to ::queuedOffer,
        "balanced keys throughout contact never become commands" to ::keysDuringContact,
        "cancel without a preceding down retires an old offer" to ::cancel,
        "canceled release retires and clears contact before returning" to ::canceledRelease,
        "focus loss clears contact and pending offers" to ::focus,
        "detach permanently retires observer and pending offers" to ::detach,
        "replacement registration rejects obsolete motion focus and detach" to ::replacement,
        "replacement reusing the same remote and palette still retires old observer" to ::samePairReplacement,
        "primary barrel suppresses remapped single click" to ::barrel,
        "fresh idle Undo after stroke executes exactly once" to ::freshIdle,
        "missing down move blocks Air until terminal" to ::missingDown,
        "finger hover and canceled secondary pointer preserve correct contact" to ::pointerKinds,
        "motion forwarding preserves identity order count and true or false result" to ::delegateResults,
        "observer failure still forwards once and fences Air" to ::observerFailure,
        "motion delegate exception propagates without a second call" to ::delegateFailure,
    )

    private class Fixture : AutoCloseable {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val dispatcher = CommandDispatcher(scope)
        var time = 1_000L
        var epoch = 1L
        var nativeReady = true
        var scheduled = 0
        val calls = mutableListOf<AirActionCommand>()
        val remote = Any()
        val palette = Any()
        private val barrel = AirActionBarrelCorrelation()
        @Suppress("UNNECESSARY_LATEINIT") // The router callback borrows the observer constructed immediately below.
        lateinit var observer: AirActionRemoteMotion
        val router = AirActionRouter(dispatcher, { time }, { AirActionBinding(epoch, AirActionContext.RemoteApp) },
            { _, _ -> nativeReady && !observer.contactActive }, { _, command -> calls += command })
        init {
            observer = create(remote, palette)
            observer.focus(remote, palette, true)
            router.configure(AirActionBindings().remap(AirActionGesture.SingleClick, AirActionCommand.Undo), true)
        }
        fun create(remote: Any, palette: Any): AirActionRemoteMotion = AirActionRemoteMotion(remote, palette, { clear ->
            epoch++
            if (clear) barrel.clear()
            router.interrupt(); barrel.restore(router, time); scheduled++
        }, { down, at -> barrel.observe(down, at); router.barrel(down, at); scheduled++ })
        fun frame(phase: AirRemoteMotionPhase, primary: Boolean = false, stylus: Boolean = true, canceled: Boolean = false) =
            AirRemoteMotionFrame(phase, 3, listOf(AirRemoteMotionPointer(3, stylus)), primary, time, canceled)
        fun observe(phase: AirRemoteMotionPhase, primary: Boolean = false, canceled: Boolean = false) {
            observer.observe(remote, palette, frame(phase, primary, canceled = canceled))
        }
        fun key(down: Long = 1_001L, up: Long = down + 1, gesture: AirActionGesture = AirActionGesture.SwipeLeft) {
            time = down; router.key(AirActionKeyEvent(gesture, AirActionKeyPhase.Down, 1, down, time))
            time = up; router.key(AirActionKeyEvent(gesture, AirActionKeyPhase.Up, 1, down, time))
        }
        fun stroke(hooked: Boolean = true) {
            time = 1_010
            if (hooked) observe(AirRemoteMotionPhase.Down)
            nativeReady = false
            time = 1_040
            if (hooked) observe(AirRemoteMotionPhase.Up)
            time = 1_100; nativeReady = true // Simulated covering ACK restores the native guard.
        }
        fun advance(at: Long = 1_252L) { time = at; router.advance() }
        fun block(): CompletableDeferred<Unit> = CompletableDeferred<Unit>().also { gate -> check(dispatcher.offer { gate.await() }) }
        override fun close() { router.close(); dispatcher.close(); scope.cancel() }
    }

    private fun negativeControl() = Fixture().use { f ->
        f.key(); f.stroke(hooked = false); f.advance()
        check(f.calls == listOf(AirActionCommand.Undo)) { "negative control must reproduce stale Undo" }
    }
    private fun staleOffer() = Fixture().use { f ->
        f.key(); f.stroke(); f.advance(); check(f.calls.isEmpty())
    }
    private fun queuedOffer() = Fixture().use { f ->
        val gate = f.block(); f.key(); f.advance()
        f.time = 1_260; f.observe(AirRemoteMotionPhase.Down)
        f.time = 1_280; f.observe(AirRemoteMotionPhase.Up)
        f.time = 1_300; f.nativeReady = true; gate.complete(Unit)
        check(f.calls.isEmpty())
    }
    private fun keysDuringContact() = Fixture().use { f ->
        f.time = 1_010; f.observe(AirRemoteMotionPhase.Down)
        // Even a permissive native flag cannot make Air eligible during contact.
        f.nativeReady = true; f.key(1_011); check(f.observer.contactActive)
        f.time = 1_040; f.observe(AirRemoteMotionPhase.Up)
        f.advance(); check(f.calls.isEmpty())
    }
    private fun cancel() = Fixture().use { f ->
        f.key(); f.time = 1_010; f.observe(AirRemoteMotionPhase.Cancel)
        check(!f.observer.contactActive); f.advance(); check(f.calls.isEmpty())
    }
    private fun canceledRelease() = Fixture().use { f ->
        f.time = 1_010; f.observe(AirRemoteMotionPhase.Down); f.key(1_011)
        f.time = 1_040; f.observe(AirRemoteMotionPhase.Up, canceled = true)
        check(!f.observer.contactActive); f.advance(); check(f.calls.isEmpty())
    }
    private fun focus() = Fixture().use { f ->
        f.key(); f.time = 1_010; f.observer.focus(f.remote, f.palette, false)
        f.observer.focus(f.remote, f.palette, true); f.advance(); check(f.calls.isEmpty())
        f.time = 1_300; f.observe(AirRemoteMotionPhase.Down)
        f.observer.focus(f.remote, f.palette, false); check(!f.observer.contactActive)
    }
    private fun detach() = Fixture().use { f ->
        f.key(); f.time = 1_010; f.observer.detach(f.remote, f.palette)
        val retired = f.epoch
        f.observer.observe(f.remote, f.palette, f.frame(AirRemoteMotionPhase.Down))
        f.observer.focus(f.remote, f.palette, true)
        check(f.epoch == retired && !f.observer.contactActive)
        f.advance(); check(f.calls.isEmpty())
    }
    private fun replacement() = Fixture().use { f ->
        val old = f.observer
        old.detach(f.remote, f.palette)
        val remote = Any(); val palette = Any()
        val next = f.create(remote, palette); f.observer = next
        next.focus(remote, palette, true)
        f.key()
        f.time = 1_010
        val before = f.epoch
        old.observe(f.remote, f.palette, f.frame(AirRemoteMotionPhase.Down))
        next.observe(f.remote, f.palette, f.frame(AirRemoteMotionPhase.Cancel))
        next.focus(f.remote, f.palette, false); next.detach(f.remote, f.palette)
        check(f.epoch == before && !next.contactActive)
        f.advance(); check(f.calls == listOf(AirActionCommand.Undo))
        f.time = 1_300; next.observe(remote, palette, f.frame(AirRemoteMotionPhase.Down))
        old.focus(f.remote, f.palette, false)
        check(next.contactActive)
    }
    private fun samePairReplacement() = Fixture().use { f ->
        val old = f.observer; old.detach(f.remote, f.palette)
        val next = f.create(f.remote, f.palette); f.observer = next
        next.focus(f.remote, f.palette, true); f.key()
        f.time = 1_010; val epoch = f.epoch
        old.observe(f.remote, f.palette, f.frame(AirRemoteMotionPhase.Down))
        old.focus(f.remote, f.palette, false); old.detach(f.remote, f.palette)
        check(f.epoch == epoch && !next.contactActive)
        f.advance(); check(f.calls == listOf(AirActionCommand.Undo))
    }
    private fun barrel() = Fixture().use { f ->
        f.time = 1_001; f.observe(AirRemoteMotionPhase.Hover, primary = true)
        f.key(1_002, gesture = AirActionGesture.SingleClick)
        f.time = 1_004; f.observe(AirRemoteMotionPhase.Hover, primary = false)
        f.advance(1_254); check(f.calls.isEmpty()); check(f.scheduled >= 2)
    }
    private fun freshIdle() = Fixture().use { f ->
        f.key(); f.stroke(); f.advance(); check(f.calls.isEmpty())
        f.key(1_300); f.advance(1_551); f.advance(1_800)
        check(f.calls == listOf(AirActionCommand.Undo))
    }
    private fun missingDown() = Fixture().use { f ->
        f.key(); f.time = 1_010; f.observe(AirRemoteMotionPhase.Move)
        check(f.observer.contactActive); f.key(1_020)
        f.time = 1_040; f.observe(AirRemoteMotionPhase.Up)
        f.advance(); check(f.calls.isEmpty())
    }
    private fun pointerKinds() = Fixture().use { f ->
        f.time = 1_001
        f.observer.observe(f.remote, f.palette, f.frame(AirRemoteMotionPhase.Down, stylus = false))
        check(!f.observer.contactActive)
        f.observe(AirRemoteMotionPhase.Hover); check(!f.observer.contactActive)
        f.observe(AirRemoteMotionPhase.Down)
        val secondary = AirRemoteMotionFrame(AirRemoteMotionPhase.PointerUp, 9,
            listOf(AirRemoteMotionPointer(3, true), AirRemoteMotionPointer(9, false)), false, f.time, true)
        f.observer.observe(f.remote, f.palette, secondary); check(f.observer.contactActive)
        f.observe(AirRemoteMotionPhase.Leave); check(!f.observer.contactActive)
    }
    private fun delegateResults() {
        for (expected in listOf(true, false)) {
            val event = Any(); val order = mutableListOf<String>(); var forwarded = 0
            val actual = observeAirRemoteMotionBeforeForward(event, { check(it === event); order += "observe" },
                { error("unexpected observation failure") }, { check(it === event); forwarded++; order += "delegate"; expected })
            check(actual == expected && forwarded == 1 && order == listOf("observe", "delegate"))
        }
    }
    private fun observerFailure() {
        for (failureCallbackThrows in listOf(false, true)) {
            val order = mutableListOf<String>(); var calls = 0
            val result = observeAirRemoteMotionBeforeForward(Any(), { order += "observe"; error("synthetic observer refusal") },
                { order += "fence"; if (failureCallbackThrows) error("synthetic fence failure") },
                { calls++; order += "delegate"; false })
            check(!result && calls == 1 && order == listOf("observe", "fence", "delegate"))
        }
    }
    private fun delegateFailure() {
        val failure = IllegalStateException("synthetic delegate failure"); var calls = 0
        try {
            observeAirRemoteMotionBeforeForward(Any(), {}, {}, { calls++; throw failure })
            error("delegate failure must propagate")
        } catch (actual: IllegalStateException) { check(actual === failure && calls == 1) }
    }
}
