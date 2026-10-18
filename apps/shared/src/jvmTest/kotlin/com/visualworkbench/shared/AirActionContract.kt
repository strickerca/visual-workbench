package com.visualworkbench.shared

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel

/** Also executable by the dependency-free focused host runner. These are policy
 * scenarios, not BLE, Android lifecycle or physical S Pen acceptance. */
internal object AirActionContract {
    fun scenarios(): Map<String, () -> Unit> = linkedMapOf(
        "all specification defaults and remaps are closed and copied" to ::defaults,
        "balanced pair executes once after resolution" to ::balanced,
        "repeat down and duplicate callback routes execute once" to ::repeats,
        "up without down and multiple never act" to ::unbalanced,
        "canceled key pair never acts" to ::canceled,
        "out of order release never acts" to ::outOfOrder,
        "lost up expires and next press works" to ::lostUp,
        "explicit double cancels preceding single" to ::double,
        "two clicks resolve to one double" to ::twoClicks,
        "second down in window prevents a premature single" to ::secondDown,
        "independent input streams never combine into a double" to ::separateStreams,
        "single following double does not add a command" to ::trailingSingle,
        "separate rapid swipe actions remain separate" to ::separateSwipes,
        "barrel before air wins" to ::barrelBefore,
        "barrel after air wins" to ::barrelAfter,
        "barrel retires an already queued air command" to ::barrelQueued,
        "held barrel and release grace suppress air" to ::barrelHold,
        "interruption before key up discards the press" to ::interruptedPress,
        "interruption retires pending and queued offers" to ::interruptedQueue,
        "replacement binding cannot adopt old input" to ::replacement,
        "execution rechecks modal and input guards" to ::guards,
        "remote context remains remote and dialog refuses" to ::contexts,
        "remap retires queued old mapping" to ::remapQueued,
        "disabled gesture and disabled module do not act" to ::disabled,
        "full serial dispatcher reports refusal without replay" to ::fullDispatcher,
        "ninth simultaneous press reports one capacity refusal without replay" to ::boundedPressed,
        "pending capacity refuses explicitly without retry" to ::boundedPending,
        "clock regression and stale events fail closed" to ::badClock,
        "late timer cannot replay old input" to ::lateTimer,
        "late dispatcher cannot replay old input" to ::lateDispatcher,
        "close retires queued work and remains closed" to ::closed,
        "onboarding keeps unknown disabled disconnected and interference distinct" to ::onboarding,
        "bounded stream receipts cannot evict and replay old keys" to ::boundedStreams,
    )

    private class Fixture(capacity: Int = 32) : AutoCloseable {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        var time = 1_000L
        var ticket: AirActionBinding? = AirActionBinding(1, AirActionContext.Canvas)
        var permitted = true
        var refusals = 0
        val calls = mutableListOf<Pair<AirActionBinding, AirActionCommand>>()
        val dispatcher = CommandDispatcher(scope, capacity)
        val router = AirActionRouter(dispatcher, { time }, { ticket }, { _, _ -> permitted },
            { bound, command -> calls += bound to command }, { refusals++ })
        init { router.configure(AirActionBindings(), true); time++ }
        fun event(gesture: AirActionGesture = AirActionGesture.SingleClick, phase: AirActionKeyPhase,
            down: Long = time, repeat: Int = 0, canceled: Boolean = false, stream: Int = 1) =
            AirActionKeyEvent(gesture, phase, stream, down, time, repeat, canceled)
        fun pair(gesture: AirActionGesture = AirActionGesture.SingleClick, stream: Int = 1) {
            val down = time
            check(router.key(event(gesture, AirActionKeyPhase.Down, down, stream = stream)))
            time++
            check(router.key(event(gesture, AirActionKeyPhase.Up, down, stream = stream)))
        }
        fun flush() { time += AirActionRouter.ResolutionMs; router.advance() }
        fun block(): CompletableDeferred<Unit> = CompletableDeferred<Unit>().also { gate -> dispatcher.offer { gate.await() } }
        fun only(command: AirActionCommand) { check(calls.map { it.second } == listOf(command)) { calls.toString() } }
        override fun close() { router.close(); dispatcher.close(); scope.cancel() }
    }

    private fun defaults() {
        val expected = AirActionCommand.entries
        val source = mutableMapOf(AirActionGesture.SingleClick to AirActionCommand.Redo)
        val bindings = AirActionBindings(source)
        source.clear()
        check(bindings.command(AirActionGesture.SingleClick) == AirActionCommand.Redo)
        check(AirActionGesture.entries.map { AirActionBindings().command(it) } == expected)
        for (gesture in AirActionGesture.entries) for (command in expected) {
            check(bindings.remap(gesture, command).command(gesture) == command)
            check(bindings.remap(gesture, null).command(gesture) == null)
        }
        check(expected.size == 8)
    }
    private fun balanced() = Fixture().use { f ->
        f.pair(); check(f.calls.isEmpty()); f.time += 249; f.router.advance(); check(f.calls.isEmpty())
        f.time++; f.router.advance(); f.only(AirActionCommand.TogglePrecisionLens)
    }
    private fun repeats() = Fixture().use { f ->
        val down = f.time; val event = f.event(phase = AirActionKeyPhase.Down)
        f.router.key(event); f.router.key(event); f.router.key(event.copy(repeatCount = 1)); f.time++
        val up = f.event(phase = AirActionKeyPhase.Up, down = down)
        f.router.key(up); f.router.key(up); f.router.key(event); f.router.key(up)
        f.flush(); f.only(AirActionCommand.TogglePrecisionLens)
    }
    private fun unbalanced() = Fixture().use { f ->
        f.router.key(f.event(phase = AirActionKeyPhase.Up)); f.router.key(f.event(phase = AirActionKeyPhase.Multiple))
        f.flush(); check(f.calls.isEmpty())
    }
    private fun canceled() = Fixture().use { f ->
        for (phase in listOf(AirActionKeyPhase.Down, AirActionKeyPhase.Up)) {
            val down = f.time
            f.router.key(f.event(phase = AirActionKeyPhase.Down, canceled = phase == AirActionKeyPhase.Down)); f.time++
            f.router.key(f.event(phase = AirActionKeyPhase.Up, down = down, canceled = phase == AirActionKeyPhase.Up)); f.time++
        }
        f.flush(); check(f.calls.isEmpty())
    }
    private fun outOfOrder() = Fixture().use { f ->
        val down = f.time
        f.router.key(f.event(phase = AirActionKeyPhase.Down).copy(downTimeMs = down - 10))
        f.router.key(f.event(phase = AirActionKeyPhase.Up).copy(downTimeMs = down - 10, eventTimeMs = down - 1))
        f.flush(); check(f.calls.isEmpty())
    }
    private fun lostUp() = Fixture().use { f ->
        repeat(8) { stream -> f.router.key(f.event(phase = AirActionKeyPhase.Down, stream = stream)) }
        f.time += 2_001; f.pair(); f.flush(); f.only(AirActionCommand.TogglePrecisionLens)
    }
    private fun double() = Fixture().use { f ->
        f.pair(); f.time += 100; f.pair(AirActionGesture.DoubleClick); f.flush(); f.only(AirActionCommand.FitRestoreView)
    }
    private fun twoClicks() = Fixture().use { f ->
        f.pair(); f.time += 100; f.pair(); f.flush(); f.only(AirActionCommand.FitRestoreView)
    }
    private fun secondDown() = Fixture().use { f ->
        f.pair(); f.time += 249; val down = f.time
        f.router.key(f.event(phase = AirActionKeyPhase.Down)); f.time++; f.router.advance(); check(f.calls.isEmpty())
        f.time += 50; f.router.key(f.event(phase = AirActionKeyPhase.Up, down = down)); f.flush(); f.only(AirActionCommand.FitRestoreView)
    }
    private fun separateStreams() = Fixture().use { f ->
        f.pair(stream = 1); f.time++; f.pair(stream = 2); f.flush()
        check(f.calls.map { it.second } == listOf(AirActionCommand.TogglePrecisionLens, AirActionCommand.TogglePrecisionLens))
    }
    private fun trailingSingle() = Fixture().use { f ->
        f.pair(AirActionGesture.DoubleClick); f.time += 100; f.pair(); f.flush(); f.only(AirActionCommand.FitRestoreView)
    }
    private fun separateSwipes() = Fixture().use { f ->
        f.pair(AirActionGesture.SwipeLeft); f.time++; f.pair(AirActionGesture.SwipeLeft); f.flush()
        check(f.calls.map { it.second } == listOf(AirActionCommand.Undo, AirActionCommand.Undo))
    }
    private fun barrelBefore() = Fixture().use { f ->
        f.router.barrel(true, f.time); f.time++; f.pair(); f.router.barrel(false, f.time); f.flush(); check(f.calls.isEmpty())
    }
    private fun barrelAfter() = Fixture().use { f ->
        f.pair(); f.time += 200; f.router.barrel(true, f.time); f.router.barrel(false, f.time); f.flush(); check(f.calls.isEmpty())
    }
    private fun barrelQueued() = Fixture().use { f ->
        val gate = f.block(); f.pair(); f.flush(); f.router.barrel(true, f.time); gate.complete(Unit); check(f.calls.isEmpty())
    }
    private fun barrelHold() = Fixture().use { f ->
        f.router.barrel(true, f.time); f.time += 800; f.pair(AirActionGesture.SwipeRight); f.flush()
        f.router.barrel(false, f.time); f.time++; f.pair(); f.flush(); check(f.calls.isEmpty())
        f.time++; f.pair(AirActionGesture.SwipeRight); f.flush(); f.only(AirActionCommand.Redo)
    }
    private fun interruptedPress() = Fixture().use { f ->
        val down = f.time; f.router.key(f.event(phase = AirActionKeyPhase.Down)); f.time++; f.router.interrupt()
        f.router.key(f.event(phase = AirActionKeyPhase.Up, down = down)); f.flush(); check(f.calls.isEmpty())
        f.time++; f.pair(); f.flush(); f.only(AirActionCommand.TogglePrecisionLens)
    }
    private fun interruptedQueue() = Fixture().use { f ->
        val gate = f.block(); f.pair(); f.flush(); f.time++; f.pair(); f.router.interrupt(); gate.complete(Unit)
        f.flush(); check(f.calls.isEmpty())
    }
    private fun replacement() = Fixture().use { f ->
        val down = f.time; f.router.key(f.event(phase = AirActionKeyPhase.Down)); f.ticket = f.ticket!!.copy(epoch = 2); f.time++
        f.router.key(f.event(phase = AirActionKeyPhase.Up, down = down)); f.flush(); check(f.calls.isEmpty())
        val gate = f.block(); f.time++; f.pair(); f.flush(); f.ticket = f.ticket!!.copy(epoch = 3); gate.complete(Unit); check(f.calls.isEmpty())
    }
    private fun guards() = Fixture().use { f ->
        f.permitted = false; f.pair(); f.flush(); check(f.calls.isEmpty())
        f.permitted = true; f.time++; f.pair(); f.permitted = false; f.flush(); check(f.calls.isEmpty())
        f.permitted = true; val gate = f.block(); f.time++; f.pair(); f.flush(); f.permitted = false
        gate.complete(Unit); check(f.calls.isEmpty())
    }
    private fun contexts() = Fixture().use { f ->
        f.ticket = AirActionBinding(2, AirActionContext.Dialog); f.pair(); f.flush(); check(f.calls.isEmpty())
        f.ticket = AirActionBinding(3, AirActionContext.RemoteApp); f.time++; f.pair(AirActionGesture.SwipeLeft); f.flush()
        f.only(AirActionCommand.Undo); check(f.calls.single().first.context == AirActionContext.RemoteApp)
    }
    private fun remapQueued() = Fixture().use { f ->
        val gate = f.block(); f.pair(); f.flush()
        f.router.configure(AirActionBindings().remap(AirActionGesture.SingleClick, AirActionCommand.NextTool), true)
        gate.complete(Unit); check(f.calls.isEmpty()); f.time++; f.pair(); f.flush(); f.only(AirActionCommand.NextTool)
    }
    private fun disabled() = Fixture().use { f ->
        f.router.configure(AirActionBindings(), false); f.time++; f.pair(); f.flush(); check(f.calls.isEmpty())
        f.router.configure(AirActionBindings().remap(AirActionGesture.SingleClick, null), true); f.time++
        f.pair(); f.flush(); check(f.calls.isEmpty()); f.time++; f.pair(AirActionGesture.DoubleClick); f.flush(); f.only(AirActionCommand.FitRestoreView)
    }
    private fun fullDispatcher() = Fixture(capacity = 1).use { f ->
        val gate = f.block(); check(f.dispatcher.offer {})
        f.pair(AirActionGesture.SwipeLeft); f.flush(); check(f.refusals == 1); gate.complete(Unit)
        f.flush(); check(f.calls.isEmpty())
    }
    private fun boundedPressed() = Fixture().use { f ->
        val downs = (0..8).map { stream -> f.event(AirActionGesture.SwipeLeft, AirActionKeyPhase.Down, stream = stream) }
        downs.take(8).forEach { check(f.router.key(it)) }
        check(f.refusals == 0 && f.calls.isEmpty())
        f.permitted = false
        check(f.router.key(downs.last().copy(stream = 9)))
        f.permitted = true
        val ninth = downs.last()
        check(f.router.key(ninth))
        check(f.router.key(ninth))
        check(f.router.key(ninth.copy(repeatCount = 1)))
        f.time++
        downs.forEach { check(f.router.key(it.copy(phase = AirActionKeyPhase.Up, eventTimeMs = f.time))) }
        f.flush()
        check(f.calls.size == 8 && f.refusals == 1) { "executed=${f.calls.size} capacity_refusals=${f.refusals}" }
        check(f.calls.all { it.second == AirActionCommand.Undo })
        // Freeing a slot does not retry the refused physical press.
        check(f.router.key(ninth.copy(eventTimeMs = f.time)))
        check(f.router.key(ninth.copy(phase = AirActionKeyPhase.Up, eventTimeMs = f.time)))
        f.flush(); check(f.calls.size == 8 && f.refusals == 1)
        f.time++; f.pair(AirActionGesture.SwipeLeft, stream = ninth.stream); f.flush()
        check(f.calls.size == 9 && f.refusals == 1)
    }
    private fun boundedPending() = Fixture().use { f ->
        repeat(17) { f.pair(AirActionGesture.SwipeLeft); f.time++ }
        check(f.refusals == 1); f.flush(); check(f.calls.size == 16); f.flush(); check(f.calls.size == 16)
    }
    private fun badClock() = Fixture().use { f ->
        f.pair(); f.time--; f.router.advance(); f.time += 500; f.router.advance(); check(f.calls.isEmpty())
        f.router.key(f.event(phase = AirActionKeyPhase.Down).copy(downTimeMs = -1))
        f.router.key(f.event(phase = AirActionKeyPhase.Down).copy(eventTimeMs = f.time + 1))
        f.flush(); check(f.calls.isEmpty())
    }
    private fun lateTimer() = Fixture().use { f -> f.pair(); f.time += 2_001; f.router.advance(); check(f.calls.isEmpty()) }
    private fun lateDispatcher() = Fixture().use { f ->
        val gate = f.block(); f.pair(); f.flush(); f.time += 2_001; gate.complete(Unit); check(f.calls.isEmpty())
    }
    private fun closed() = Fixture().use { f ->
        val gate = f.block(); f.pair(); f.flush(); f.router.close(); f.router.close(); gate.complete(Unit)
        f.router.configure(AirActionBindings(), true); f.time++; f.pair(); f.flush(); check(f.calls.isEmpty())
    }
    private fun onboarding() {
        val ready = AirActionSetup(AirActionSupport.Supported, true, true, false)
        check(ready.routingReady); check(ready.message.contains("unverified"))
        check(!AirActionSetup(keyDeliveryObserved = true).routingReady)
        for (setup in listOf(AirActionSetup(), ready.copy(support = AirActionSupport.Unsupported), ready.copy(enabled = false),
            ready.copy(connected = false), ready.copy(airCommandInterference = true), ready.copy(airCommandInterference = null))) {
            check(!setup.routingReady); check(setup.message.isNotBlank())
        }
        check(ready.copy(connected = false).message.contains("Reconnect"))
        check(ready.copy(airCommandInterference = true).message.contains("Air Command"))
        check(ready.copy(keyDeliveryObserved = true).message.contains("acceptance still needs"))
    }
    private fun boundedStreams() = Fixture().use { f ->
        val oldDown = f.time
        repeat(32) { stream -> f.pair(AirActionGesture.SwipeLeft, stream); f.time++ }
        val before = f.refusals; f.pair(AirActionGesture.SwipeLeft, 33); check(f.refusals > before)
        f.router.key(f.event(AirActionGesture.SwipeLeft, AirActionKeyPhase.Down, oldDown, stream = 0))
        f.router.key(f.event(AirActionGesture.SwipeLeft, AirActionKeyPhase.Up, oldDown, stream = 0))
        f.flush(); check(f.calls.size == 16)
    }
}
