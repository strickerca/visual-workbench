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
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel

/** Actual callback dispatch function and router; no framework Window/View or
 * ComponentActivity is instantiated by these standalone host scenarios. */
internal object AirActionWindowKeyContract {
    fun scenarios(): Map<String, () -> Unit> = linkedMapOf(
        "consumed key is observed once before view dispatch and never forwarded" to ::consumed,
        "unconsumed key preserves identity and fallback true or false exactly once" to ::fallback,
        "retired callback never observes and forwards exactly once" to ::retired,
        "fallback exception propagates without retry" to ::fallbackFailure,
        "observer exception preserves prior behavior without a fallback retry" to ::observerFailure,
        "actual router consumes balanced action keys and executes once" to ::balanced,
        "lifecycle retirement between down and up prevents a command" to ::interrupted,
        "Activity refuses remote context while Dialog owns the action event" to ::remoteContext,
    )

    private fun consumed() {
        val event = Any(); var observed = 0; var delegated = 0
        val result = dispatchAirKeyBeforeFallback(event, true, { check(it === event); observed++; true },
            { delegated++; false })
        check(result && observed == 1 && delegated == 0)
    }
    private fun fallback() {
        for (expected in listOf(true, false)) {
            val event = Any(); val order = mutableListOf<String>()
            val result = dispatchAirKeyBeforeFallback(event, true, { check(it === event); order += "observe"; false },
                { check(it === event); order += "fallback"; expected })
            check(result == expected && order == listOf("observe", "fallback"))
        }
    }
    private fun retired() {
        val event = Any(); var observed = 0; var delegated = 0
        val result = dispatchAirKeyBeforeFallback(event, false, { observed++; true },
            { check(it === event); delegated++; false })
        check(!result && observed == 0 && delegated == 1)
    }
    private fun fallbackFailure() {
        val failure = IllegalStateException("synthetic fallback failure"); var observed = 0; var delegated = 0
        try {
            dispatchAirKeyBeforeFallback(Any(), true, { observed++; false }, { delegated++; throw failure })
            error("expected original fallback failure")
        } catch (actual: IllegalStateException) { check(actual === failure && observed == 1 && delegated == 1) }
    }
    private fun observerFailure() {
        val failure = IllegalStateException("synthetic receiver failure"); var delegated = 0
        try {
            dispatchAirKeyBeforeFallback(Any(), true, { throw failure }, { delegated++; true })
            error("expected original receiver failure")
        } catch (actual: IllegalStateException) { check(actual === failure && delegated == 0) }
    }

    private class Fixture : AutoCloseable {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val dispatcher = CommandDispatcher(scope)
        var time = 1_000L
        var epoch = 1L
        var active = true
        var local = true
        var observed = 0
        var delegated = 0
        val calls = mutableListOf<AirActionCommand>()
        val router = AirActionRouter(dispatcher, { time }, { AirActionBinding(epoch, if (local) AirActionContext.Canvas else AirActionContext.RemoteApp) },
            { _, _ -> true }, { _, command -> calls += command })
        init { router.configure(AirActionBindings(), true) }
        fun event(phase: AirActionKeyPhase) = AirActionKeyEvent(AirActionGesture.SwipeLeft, phase, 1, 1_001, time)
        fun dispatch(event: AirActionKeyEvent): Boolean = dispatchAirKeyBeforeFallback(event, active,
            { observed++; if (local) router.key(it) else false }, { delegated++; false })
        fun flush() { time = 1_252; router.advance() }
        override fun close() { router.close(); dispatcher.close(); scope.cancel() }
    }
    private fun balanced() = Fixture().use { f ->
        f.time = 1_001; check(f.dispatch(f.event(AirActionKeyPhase.Down)))
        f.time = 1_002; val up = f.event(AirActionKeyPhase.Up)
        check(f.dispatch(up)); check(f.dispatch(up)); f.flush()
        check(f.calls == listOf(AirActionCommand.Undo) && f.observed == 3 && f.delegated == 0)
    }
    private fun interrupted() = Fixture().use { f ->
        f.time = 1_001; f.dispatch(f.event(AirActionKeyPhase.Down))
        f.epoch++; f.router.interrupt(); f.active = false
        f.time = 1_002; check(!f.dispatch(f.event(AirActionKeyPhase.Up)))
        f.flush(); check(f.calls.isEmpty() && f.observed == 1 && f.delegated == 1)
    }
    private fun remoteContext() = Fixture().use { f ->
        f.local = false
        for (phase in listOf(AirActionKeyPhase.Down, AirActionKeyPhase.Up)) {
            f.time = if (phase == AirActionKeyPhase.Down) 1_001 else 1_002
            val event = f.event(phase)
            check(!f.dispatch(event)) // Current Activity refuses the Dialog's context.
            check(dispatchAirKeyBeforeFallback(event, true, f.router::key, { error("declared Dialog key must be consumed") }))
        }
        f.flush(); check(f.calls == listOf(AirActionCommand.Undo) && f.delegated == 2)
    }
}
